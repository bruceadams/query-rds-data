use clap::{AppSettings::ColoredHelp, ArgEnum, Clap};
use exitfailure::ExitFailure;
use futures::join;
use futures::prelude::*;
use log::info;
use rusoto_core::{region::Region, RusotoError};
use rusoto_rds::{DBCluster, DescribeDBClustersError, DescribeDBClustersMessage, Rds, RdsClient};
use rusoto_rds_data::{
    ExecuteStatementRequest, ExecuteStatementResponse, Field, RdsData, RdsDataClient,
    ResultSetOptions,
};
use rusoto_secretsmanager::{
    ListSecretsError, ListSecretsRequest, SecretListEntry, SecretsManager, SecretsManagerClient,
};
use serde::ser::SerializeMap;
use serde::{Serialize, Serializer};
use serde_json::Value;
use snafu::Snafu;
use std::{env, io::stdout, io::Write, str::FromStr};

#[derive(ArgEnum, Copy, Clone, Debug, PartialEq, enum_utils::FromStr)]
#[enumeration(case_insensitive)]
enum Format {
    Csv,
    Cooked,
    Raw,
}

/// Query an Amazon RDS database
#[derive(Clap, Clone, Debug)]
#[clap(global_setting = ColoredHelp)]
struct MyArgs {
    /// AWS source profile to use. This name references an entry in ~/.aws/config
    #[clap(long = "aws-profile", short = 'p')]
    profile: Option<String>,

    /// AWS region to target.
    #[clap(
        default_value = "us-east-1",
        env = "AWS_DEFAULT_REGION",
        long = "aws-region",
        short = 'r'
    )]
    region: String,

    /// RDS database identifier.
    #[clap(env = "AWS_RDS_CLUSTER", long = "db-cluster-identifier", short = 'c')]
    db_id: Option<String>,

    /// RDS user identifier (really the AWS secret identifier).
    #[clap(env = "AWS_RDS_USER", long = "db-user-identifier", short = 'u')]
    user_id: Option<String>,

    /// Output format. One of "csv", "cooked", "raw"
    #[clap(default_value = "csv", long = "format", short = 'f')]
    format: String,

    /// Database name.
    #[clap(env = "AWS_RDS_DATABASE", long = "database", short = 'd')]
    database: Option<String>,

    /// SQL query.
    query: String,

    /// Increase logging verbosity (-v, -vv, -vvv, etc)
    #[clap(short = 'v', long = "verbose", parse(from_occurrences))]
    verbose: usize,
}

#[derive(Debug, Snafu)]
enum Error {
    #[snafu(display("Failed to lookup clusters: {}", source))]
    DBClusterLookup {
        source: RusotoError<DescribeDBClustersError>,
    },
    #[snafu(display("Failed to find any RDS databases"))]
    DBClusterLookupEmpty {},
    #[snafu(display("No DBs found"))]
    DBClusterEmpty {},
    #[snafu(display(
        "No DB matched \"{}\", available ids are {:?}",
        db_cluster_identifier,
        available_ids
    ))]
    DBClusterNoMatch {
        db_cluster_identifier: String,
        available_ids: Vec<String>,
    },
    #[snafu(display("Multiple DBs found, please specify one of {:?}", available_ids))]
    DBClusterMultiple { available_ids: Vec<String> },

    #[snafu(display("Failed to lookup secrets: {}", source))]
    SecretLookup {
        source: RusotoError<ListSecretsError>,
    },
    #[snafu(display("Failed to find any secrets"))]
    SecretNotFound {},
    #[snafu(display("No DB user secrets found"))]
    SecretsUsersEmpty {},
    #[snafu(display(
        "No DB user matched \"{}\", available users are {:?}",
        db_user_id,
        available_ids
    ))]
    SecretsUsersNoMatch {
        db_user_id: String,
        available_ids: Vec<String>,
    },
    #[snafu(display("Multiple DB users found, please specify one of {:?}", available_ids))]
    SecretsUsersMultiple { available_ids: Vec<String> },
}

struct MyArns {
    aws_secret_store_arn: String,
    db_cluster_or_instance_arn: String,
}

/// Extract a name for each column
fn format_header<'a>(result: &'a ExecuteStatementResponse) -> impl Iterator<Item = &'a str> {
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

/// The incoming Field data here is unfortunate.
/// The actual data structure _allows_ for multiple values.
/// I presume that, at least normally, only one value will be set.
/// This code will _work_, if maybe not well, even if more than one
/// value is set.
fn format_value(value: &Field) -> String {
    let mut string = String::new();
    if let Some(ref array_value) = value.array_value {
        string.push_str(&format!("{:?}", array_value));
    };
    if let Some(ref blob_value) = value.blob_value {
        string.push_str(&format!("{:?}", blob_value));
    };
    if let Some(ref boolean_value) = value.boolean_value {
        string.push_str(&format!("{:?}", boolean_value));
    };
    if let Some(ref double_value) = value.double_value {
        string.push_str(&format!("{:?}", double_value));
    };
    if let Some(ref _is_null) = value.is_null {
        string.push_str("NULL");
    };
    if let Some(ref long_value) = value.long_value {
        string.push_str(&format!("{:?}", long_value));
    };
    if let Some(ref string_value) = value.string_value {
        string.push_str(&string_value);
    };
    string
}

fn one_row(values: &[Field]) -> impl Iterator<Item = String> + '_ {
    values.iter().map(|value| format_value(&value))
}

/// Return an iterator of iterators of strings
fn format_rows(
    result: &ExecuteStatementResponse,
) -> impl Iterator<Item = impl Iterator<Item = String> + '_> {
    // This seems pretty crazed...
    result
        .records
        .as_ref()
        .map_or(&[][..], |x| &**x)
        .iter()
        .map(|record| one_row(record))
}

fn cluster_ids(db_clusters: &[DBCluster]) -> Vec<String> {
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
    db_clusters: &[DBCluster],
) -> Result<DBCluster, Error> {
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
    region: &Region,
    requested_db_cluster_identifier: &Option<String>,
    requested_user_id: &Option<String>,
) -> Result<MyArns, Error> {
    let describe_db_clusters_message = DescribeDBClustersMessage::default();
    let list_secrets_request = ListSecretsRequest::default();

    let rds_client = RdsClient::new(region.clone());
    let secrets_manager_client = SecretsManagerClient::new(region.clone());
    info!("{:?}", describe_db_clusters_message);
    info!("{:?}", list_secrets_request);
    let fut1 = rds_client
        .describe_db_clusters(describe_db_clusters_message)
        .map_err(|e| Error::DBClusterLookup { source: e });
    let fut2 = secrets_manager_client
        .list_secrets(list_secrets_request)
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
            &requested_user_id,
            &secret_list,
        )?,
        None => return Err(Error::SecretNotFound {}),
    };
    Ok(MyArns {
        aws_secret_store_arn: secret_list_entry.arn.unwrap(),
        db_cluster_or_instance_arn: db_cluster.db_cluster_arn.unwrap(),
    })
}

fn csv_output(result: &ExecuteStatementResponse) -> Result<(), ExitFailure> {
    if let Some(number_of_records_updated) = result.number_of_records_updated {
        if number_of_records_updated > 0 || result.column_metadata.is_none() {
            println!("number_of_records_updated: {}", number_of_records_updated)
        }
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
/// We use a vector of tuples, not some map, to preserve order and retain fields
/// which happen to have the same name. Many consumers of the JSON output from
/// here may struggle when field names are repeated, but I rather output them as
/// given instead of silently dropping them.
struct Record {
    // Serialize as a map, preserving order and allowing for repeated keys.
    #[serde(serialize_with = "SerdeRecord::serialize_record", default, flatten)]
    record: Vec<(String, Value)>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
struct CookedResponse {
    /// The number of records updated by the request.
    #[serde(rename = "numberOfRecordsUpdated")]
    #[serde(skip_serializing_if = "Option::is_none")]
    number_of_records_updated: Option<i64>,

    /// The records returned by the SQL statement.
    pub records: Vec<Record>,
}

fn field_value(field: &Field) -> Value {
    if let Some(ref boolean_value) = field.boolean_value {
        Value::from(*boolean_value)
    } else if let Some(ref double_value) = field.double_value {
        Value::from(*double_value)
    } else if let Some(ref _is_null) = field.is_null {
        Value::Null
    } else if let Some(ref long_value) = field.long_value {
        Value::from(*long_value)
    } else if let Some(ref string_value) = field.string_value {
        Value::from(string_value.clone())
    } else {
        // Punt!
        Value::Null
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

fn cook_response(result: &ExecuteStatementResponse) -> CookedResponse {
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

fn cooked_output(result: &ExecuteStatementResponse) -> Result<(), ExitFailure> {
    serde_json::to_writer_pretty(stdout(), &cook_response(result))?;
    // We'd like to write out a final newline. Ignore any failure to do so.
    let _result = stdout().write(b"\n");
    Ok(())
}

fn raw_output(result: &ExecuteStatementResponse) -> Result<(), ExitFailure> {
    serde_json::to_writer_pretty(stdout(), result)?;
    // We'd like to write out a final newline. Ignore any failure to do so.
    let _result = stdout().write(b"\n");
    Ok(())
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
    // FIXME: Better to give an error message here.
    let output_format = args.format.parse().unwrap_or(Format::Csv);
    if let Some(aws_profile) = args.profile {
        env::set_var("AWS_PROFILE", aws_profile);
    };
    let region = Region::from_str(&args.region)?;
    let rds_data_client = RdsDataClient::new(region.clone());

    let my_arns = get_arns(&region, &args.db_id, &args.user_id).await?;

    let execute_sql_request = ExecuteStatementRequest {
        include_result_metadata: Some(true),
        resource_arn: my_arns.db_cluster_or_instance_arn,
        secret_arn: my_arns.aws_secret_store_arn,
        sql: args.query,
        database: args.database,
        result_set_options: Some(ResultSetOptions {
            decimal_return_type: Some("STRING".to_string()),
        }),
        ..Default::default()
    };

    info!("{:?}", execute_sql_request);
    // The result that comes back is fairly intense.
    // I don't know how to take it apart in a non-tedious way.
    let execute_statement_response = rds_data_client
        .execute_statement(execute_sql_request)
        .await?;
    info!("{:?}", execute_statement_response);
    match output_format {
        Format::Csv => csv_output(&execute_statement_response)?,
        Format::Cooked => cooked_output(&execute_statement_response)?,
        Format::Raw => raw_output(&execute_statement_response)?,
    };
    Ok(())
}
