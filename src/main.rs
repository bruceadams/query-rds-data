use std::time::Duration;

use aws_config::{identity::IdentityCache, BehaviorVersion, SdkConfig};
use aws_sdk_rds::{
    error::SdkError, operation::describe_db_clusters::DescribeDBClustersError, types::DbCluster,
};
use aws_sdk_rdsdata::{
    operation::execute_statement::ExecuteStatementOutput,
    types::{DecimalReturnType, Field, ResultSetOptions},
};
use aws_sdk_secretsmanager::{operation::list_secrets::ListSecretsError, types::SecretListEntry};
use aws_types::region::Region;
use clap::{crate_description, crate_version, ArgAction, Parser, ValueEnum};
use exitfailure::ExitFailure;
use futures::join;
use futures::prelude::*;
use log::info;
use serde::{ser::SerializeMap, Serialize, Serializer};
use serde_json::Value;
use snafu::Snafu;
use std::io::{stdout, Write};

#[derive(Copy, Clone, Debug, PartialEq, ValueEnum)]
enum Format {
    /// CSV output, including a header line.
    Csv,
    /// An array of JSON Objects, {"field_name": field_value, …}.
    Json,
}

#[derive(Clone, Debug, Parser)]
#[clap(about=crate_description!(), version=crate_version!())]
struct MyArgs {
    /// AWS source profile to use. This name references an entry in ~/.aws/config
    #[clap(env = "AWS_PROFILE", long, short)]
    profile: Option<String>,

    /// AWS region to target.
    #[clap(env = "AWS_REGION", long, short)]
    region: Option<String>,

    /// RDS cluster identifier.
    #[clap(env = "AWS_RDS_CLUSTER", long = "db-cluster-identifier", short)]
    cluster_id: Option<String>,

    /// RDS user identifier (really the AWS secret identifier).
    #[clap(env = "AWS_RDS_USER", long = "db-user-identifier", short)]
    user_id: Option<String>,

    /// Output format.
    #[clap(value_enum, default_value = "csv", long, short)]
    format: Format,

    /// Database name.
    #[clap(env = "AWS_RDS_DATABASE", long, short)]
    database: Option<String>,

    /// SQL query.
    query: String,

    /// Increase logging verbosity (-v, -vv, -vvv, etc)
    #[clap(action = ArgAction::Count, long, short)]
    verbose: u8,
}

#[derive(Debug, Snafu)]
enum Error {
    #[snafu(display("Failed to lookup clusters: {}", source))]
    DBClusterLookup {
        source: SdkError<DescribeDBClustersError>,
    },
    #[snafu(display("Failed to find any RDS clusters"))]
    DBClusterLookupEmpty {},
    #[snafu(display("No clusters found"))]
    DBClusterEmpty {},
    #[snafu(display(
        "No cluster matched \"{}\", available ids are {:?}",
        db_cluster_identifier,
        available_ids
    ))]
    DBClusterNoMatch {
        db_cluster_identifier: String,
        available_ids: Vec<String>,
    },
    #[snafu(display("Multiple clusters found, please specify one of {:?}", available_ids))]
    DBClusterMultiple { available_ids: Vec<String> },

    #[snafu(display("Failed to lookup secrets: {}", source))]
    SecretLookup { source: SdkError<ListSecretsError> },
    #[snafu(display("Failed to find any secrets"))]
    SecretNotFound {},
    #[snafu(display("No cluster user secrets found"))]
    SecretsUsersEmpty {},
    #[snafu(display(
        "No cluster user matched \"{}\", available users are {:?}",
        db_user_id,
        available_ids
    ))]
    SecretsUsersNoMatch {
        db_user_id: String,
        available_ids: Vec<String>,
    },
    #[snafu(display(
        "Multiple cluster users found, please specify one of {:?}",
        available_ids
    ))]
    SecretsUsersMultiple { available_ids: Vec<String> },
}

struct MyArns {
    aws_secret_store_arn: String,
    db_cluster_or_instance_arn: String,
}

/// Extract a name for each column
fn format_header<'a>(result: &'a ExecuteStatementOutput) -> impl Iterator<Item = &'a str> {
    // This seems pretty crazed...
    result
        .column_metadata
        .as_ref()
        .map_or(&[][..], |x| &**x)
        .iter()
        .map::<&'a str, _>(|column| {
            if let Some(ref label) = column.label {
                label
            } else if let Some(ref name) = column.name {
                name
            } else {
                "?"
            }
        })
}

fn format_value(value: &Field) -> String {
    match value {
        Field::ArrayValue(inner) => format!("{:?}", *inner),
        Field::BlobValue(inner) => format!("{:?}", *inner),
        Field::BooleanValue(inner) => format!("{:?}", *inner),
        Field::DoubleValue(inner) => format!("{:?}", *inner),
        Field::IsNull(_) => "NULL".to_owned(),
        Field::LongValue(inner) => format!("{:?}", *inner),
        Field::StringValue(inner) => inner.to_owned(),
        _ => "UNKNOWN".to_owned(), // punt!!
    }
}

fn one_row(values: &[Field]) -> impl Iterator<Item = String> + '_ {
    values.iter().map(format_value)
}

/// Return an iterator of iterators of strings
fn format_rows(
    result: &ExecuteStatementOutput,
) -> impl Iterator<Item = impl Iterator<Item = String> + '_> {
    // This seems pretty crazed...
    result
        .records
        .as_ref()
        .map_or(&[][..], |x| &**x)
        .iter()
        .map(|record| one_row(record))
}

fn cluster_ids(db_clusters: &[DbCluster]) -> Vec<String> {
    db_clusters
        .iter()
        .map(|db_cluster| {
            db_cluster
                .db_cluster_identifier
                .as_ref()
                .unwrap_or(&"".to_string())
                .to_owned()
        })
        .collect()
}

fn my_cluster(
    requested_db_cluster_identifier: &Option<String>,
    db_clusters: &[DbCluster],
) -> Result<DbCluster, Error> {
    match requested_db_cluster_identifier {
        Some(requested_db_cluster_identifier) => {
            for db_cluster in db_clusters {
                if let Some(ref db_cluster_identifier) = db_cluster.db_cluster_identifier {
                    // Since this is an exact match, we assume there is only one.
                    if requested_db_cluster_identifier == db_cluster_identifier {
                        return Ok(db_cluster.to_owned());
                    }
                }
            }
            Err(Error::DBClusterNoMatch {
                db_cluster_identifier: requested_db_cluster_identifier.to_owned(),
                available_ids: cluster_ids(db_clusters),
            })
        }
        None => {
            match db_clusters.len() {
                // There is exactly one: go ahead and use it.
                1 => Ok(db_clusters[0].to_owned()),
                0 => Err(Error::DBClusterEmpty {}),
                _ => Err(Error::DBClusterMultiple {
                    available_ids: cluster_ids(db_clusters),
                }),
            }
        }
    }
}

fn secrets_for_db<'a>(
    requested_db_cluster_resource_id: &str,
    secret_list: &'a [SecretListEntry],
) -> Vec<&'a SecretListEntry> {
    // I don't know if this is a universal naming standard for secrets.
    // If not, this code is badly wrong.
    let name_starts_with =
        "rds-db-credentials/".to_string() + requested_db_cluster_resource_id + "/";
    secret_list
        .iter()
        .filter(|secret_list_entry| match secret_list_entry.name {
            Some(ref name) => name.starts_with(&name_starts_with).to_owned(),
            None => false,
        })
        .collect()
}

fn user_id_from_secret(secret_list_entry: &SecretListEntry) -> String {
    match secret_list_entry.name {
        Some(ref name) => name.splitn(3, '/').last().unwrap_or("").to_string(),
        None => "".to_string(),
    }
}

fn user_names(secret_list: &[&SecretListEntry]) -> Vec<String> {
    secret_list
        .iter()
        .map(|entry| user_id_from_secret(entry))
        .collect()
}

fn my_secret(
    requested_db_cluster_resource_id: &str,
    requested_db_user_id: &Option<String>,
    secret_list: &[SecretListEntry],
) -> Result<SecretListEntry, Error> {
    let db_secrets = secrets_for_db(requested_db_cluster_resource_id, secret_list);

    match requested_db_user_id {
        Some(requested_db_user_id) => {
            for secret_list_entry in &db_secrets {
                if let Some(ref name) = secret_list_entry.name {
                    if name.ends_with(requested_db_user_id) {
                        // Since this is an exact match, we assume there is only one.
                        return Ok((*secret_list_entry).to_owned());
                    }
                }
            }
            Err(Error::SecretsUsersNoMatch {
                db_user_id: requested_db_user_id.to_owned(),
                available_ids: user_names(&db_secrets),
            })
        }
        None => {
            match db_secrets.len() {
                // There is exactly one: go ahead and use it.
                1 => Ok(db_secrets[0].to_owned()),
                0 => Err(Error::SecretsUsersEmpty {}),
                _ => Err(Error::SecretsUsersMultiple {
                    available_ids: user_names(&db_secrets),
                }),
            }
        }
    }
}

async fn get_arns(
    aws_sdk_config: &SdkConfig,
    requested_db_cluster_identifier: &Option<String>,
    requested_user_id: &Option<String>,
) -> Result<MyArns, Error> {
    let rds_client = aws_sdk_rds::Client::new(aws_sdk_config);
    let secrets_manager_client = aws_sdk_secretsmanager::Client::new(aws_sdk_config);

    let fut1 = rds_client
        .describe_db_clusters()
        .max_records(100)
        .send()
        .map_err(|e| Error::DBClusterLookup { source: e });
    let fut2 = secrets_manager_client
        .list_secrets()
        .max_results(100)
        .send()
        .map_err(|e| Error::SecretLookup { source: e });

    let (db_cluster_message, list_secrets_response) = join!(fut1, fut2);
    info!("{:?}", db_cluster_message);
    info!("{:?}", list_secrets_response);
    let db_cluster = match db_cluster_message?.db_clusters {
        Some(db_clusters) => my_cluster(requested_db_cluster_identifier, &db_clusters)?,
        None => return Err(Error::DBClusterLookupEmpty {}),
    };
    let secret_list_entry = match list_secrets_response?.secret_list {
        Some(secret_list) => my_secret(
            &db_cluster.db_cluster_resource_id.unwrap(),
            requested_user_id,
            &secret_list,
        )?,
        None => return Err(Error::SecretNotFound {}),
    };
    Ok(MyArns {
        aws_secret_store_arn: secret_list_entry.arn.unwrap(),
        db_cluster_or_instance_arn: db_cluster.db_cluster_arn.unwrap(),
    })
}

fn csv_output(result: &ExecuteStatementOutput) -> Result<(), ExitFailure> {
    if result.number_of_records_updated > 0 || result.column_metadata.is_none() {
        println!(
            "number_of_records_updated: {}",
            result.number_of_records_updated
        )
    }
    let mut wtr = csv::Writer::from_writer(stdout());
    wtr.write_record(format_header(result))?;
    for row in format_rows(result) {
        wtr.write_record(row)?;
    }
    Ok(())
}

pub trait SerdeRecord: Sized {
    fn serialize_record<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer;
}

impl SerdeRecord for Vec<(String, Value)> {
    fn serialize_record<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(self.len()))?;
        for (k, v) in self {
            map.serialize_entry(k, v)?;
        }
        map.end()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
/// We use a vector of tuples, not some map, to preserve order and retain
/// fields which happen to have the same name. Many consumers of the JSON
/// output from here may struggle when field names are repeated, but I prefer
/// outputting them as given instead of silently dropping them.
struct Record {
    // Serialize as a map, preserving order and allowing for repeated keys.
    #[serde(serialize_with = "SerdeRecord::serialize_record", default, flatten)]
    record: Vec<(String, Value)>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
struct CookedResponse {
    /// The number of records updated by the request.
    #[serde(rename = "numberOfRecordsUpdated")]
    number_of_records_updated: i64,

    /// The records returned by the SQL statement.
    pub records: Vec<Record>,
}

fn field_value(field: &Field) -> Value {
    match field {
        Field::ArrayValue(_array_value) => Value::Null, // punt!!
        Field::BlobValue(_blob_value) => Value::Null,   // punt!!
        Field::BooleanValue(boolean_value) => Value::from(*boolean_value),
        Field::DoubleValue(double_value) => Value::from(*double_value),
        Field::IsNull(_) => Value::Null,
        Field::LongValue(long_value) => Value::from(*long_value),
        Field::StringValue(string_value) => Value::from(string_value.clone()),
        _ => Value::Null, // punt!!
    }
}

fn annotate_fields(header: &[&str], record: &[Field]) -> Record {
    Record {
        record: header
            .iter()
            .zip(record.iter())
            .map(|(key, field)| ((*key).to_owned(), field_value(field)))
            .collect(),
    }
}

fn cook_response(result: &ExecuteStatementOutput) -> CookedResponse {
    let header: Vec<&str> = format_header(result).collect();
    CookedResponse {
        number_of_records_updated: result.number_of_records_updated,
        records: result
            .records
            .as_ref()
            .map_or(&[][..], |x| &**x)
            .iter()
            .map(|record| annotate_fields(&header, record))
            .collect(),
    }
}

fn cooked_output(result: &ExecuteStatementOutput) -> Result<(), ExitFailure> {
    serde_json::to_writer_pretty(stdout(), &cook_response(result))?;
    // We'd like to write out a final newline. Ignore any failure to do so.
    let _result = stdout().write(b"\n");
    Ok(())
}

async fn aws_sdk_config(args: &MyArgs) -> SdkConfig {
    let base = aws_config::defaults(BehaviorVersion::latest()).identity_cache(
        IdentityCache::lazy()
            .load_timeout(Duration::from_secs(90))
            .build(),
    );
    let with_profile = match &args.profile {
        None => base,
        Some(profile_name) => base.profile_name(profile_name),
    };
    let with_overrides = match &args.region {
        None => with_profile,
        Some(region_name) => with_profile.region(Region::new(region_name.clone())),
    };
    with_overrides.load().await
}

#[tokio::main]
async fn main() -> Result<(), ExitFailure> {
    let args = MyArgs::parse();
    loggerv::Logger::new()
        .output(&log::Level::Info, loggerv::Output::Stderr)
        .output(&log::Level::Debug, loggerv::Output::Stderr)
        .output(&log::Level::Trace, loggerv::Output::Stderr)
        .verbosity(args.verbose as u64)
        .init()?;
    let output_format = args.format;
    let config = aws_sdk_config(&args).await;
    let MyArns {
        aws_secret_store_arn: secret_arn,
        db_cluster_or_instance_arn: resource_arn,
    } = get_arns(&config, &args.cluster_id, &args.user_id).await?;
    let rds_data_client = aws_sdk_rdsdata::Client::new(&config);
    let result_set_options = ResultSetOptions::builder()
        .decimal_return_type(DecimalReturnType::String)
        .build();
    let execute_statement_output = rds_data_client
        .execute_statement()
        .set_database(args.database)
        .include_result_metadata(true)
        .resource_arn(resource_arn)
        .result_set_options(result_set_options)
        .secret_arn(secret_arn)
        .sql(args.query)
        .send()
        .await?;
    info!("{:?}", execute_statement_output);
    match output_format {
        Format::Csv => csv_output(&execute_statement_output),
        Format::Json => cooked_output(&execute_statement_output),
    }
}
