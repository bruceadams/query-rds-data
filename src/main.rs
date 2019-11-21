use exitfailure::ExitFailure;
use futures::prelude::*;
use log::{error, info, warn};
use rusoto_core::{region::Region, RusotoError};
use rusoto_rds::{DBCluster, DescribeDBClustersError, DescribeDBClustersMessage, Rds, RdsClient};
use rusoto_rds_data::{
    ExecuteSqlRequest, RdsData, RdsDataClient, ResultFrame, ResultSetMetadata, SqlStatementResult,
    Value,
};
use rusoto_secretsmanager::{
    ListSecretsError, ListSecretsRequest, SecretListEntry, SecretsManager, SecretsManagerClient,
};
use snafu::Snafu;
use std::{env, io::stdout, str::FromStr};
use structopt::{clap::AppSettings::ColoredHelp, StructOpt};
use tokio::runtime::Runtime;

const EMPTY_RESULT_FRAME: ResultFrame = ResultFrame {
    records: None,
    result_set_metadata: None,
};

const EMPTY_RESULT_SET_METADATA: ResultSetMetadata = ResultSetMetadata {
    column_count: None,
    column_metadata: None,
};

/// Query an Amazon RDS database
#[derive(Debug, StructOpt)]
#[structopt(global_settings(&[ColoredHelp]))]
struct MyArgs {
    /// AWS source profile to use. This name references an entry in ~/.aws/credentials
    #[structopt(env = "AWS_PROFILE", long = "aws-profile", short = "p")]
    profile: String,

    /// AWS region to target.
    #[structopt(
        default_value = "us-east-1",
        env = "AWS_DEFAULT_REGION",
        long = "aws-region",
        short = "r"
    )]
    region: String,

    /// RDS database identifier.
    #[structopt(long = "db-cluster-identifier", short = "c")]
    db_id: Option<String>,

    /// RDS user identifier (really the AWS secret identifier).
    #[structopt(long = "db-user-identifier", short = "u")]
    user_id: Option<String>,

    /// SQL query.
    query: String,

    /// Silence all output
    #[structopt(short = "q", long = "quiet")]
    quiet: bool,
    /// Verbose mode (-v, -vv, -vvv, etc)
    #[structopt(short = "v", long = "verbose", parse(from_occurrences))]
    verbose: usize,
    /// Timestamp (sec, ms, ns, none)
    #[structopt(short = "t", long = "timestamp")]
    ts: Option<stderrlog::Timestamp>,
}

#[derive(Debug, Snafu)]
enum Error {
    // DescribeDBClustersError, ListSecretsError
    #[snafu(display("Failed to find cluster: {}", source))]
    DBClusterLookup {
        source: RusotoError<DescribeDBClustersError>,
    },
    #[snafu(display("Failed to find secret: {}", source))]
    SecretLookup {
        source: RusotoError<ListSecretsError>,
    },
    #[snafu(display("Failed to find secret"))]
    SecretNotFound {},
    #[snafu(display("Failed to find DB {}", db_cluster_identifier))]
    DBNotFound { db_cluster_identifier: String },
}

struct MyArns {
    aws_secret_store_arn: String,
    db_cluster_or_instance_arn: String,
}

/// Extract a name for each column
fn format_header<'a>(result: &'a SqlStatementResult) -> impl Iterator<Item = &'a str> {
    // This seems pretty crazed...
    result
        .result_frame
        .as_ref()
        .unwrap_or(&EMPTY_RESULT_FRAME)
        .result_set_metadata
        .as_ref()
        .unwrap_or(&EMPTY_RESULT_SET_METADATA)
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
/// The incoming Value data here is unfortunate.
/// The actual data structure _allows_ for multiple values.
/// I presume that at least normally, only one value will be set.
/// This code will _work_, if maybe not well, even if more than one
/// value is set.
fn format_value(value: &Value) -> String {
    let mut string = String::new();
    if let Some(ref array_values) = value.array_values {
        string.push_str(&format!("{:?}", array_values));
    };
    if let Some(ref big_int_value) = value.big_int_value {
        string.push_str(&format!("{:?}", big_int_value));
    };
    if let Some(ref bit_value) = value.bit_value {
        string.push_str(&format!("{:?}", bit_value));
    };
    if let Some(ref blob_value) = value.blob_value {
        string.push_str(&format!("{:?}", blob_value));
    };
    if let Some(ref double_value) = value.double_value {
        string.push_str(&format!("{:?}", double_value));
    };
    if let Some(ref int_value) = value.int_value {
        string.push_str(&format!("{:?}", int_value));
    };
    if let Some(ref _is_null) = value.is_null {
        string.push_str("NULL");
    };
    if let Some(ref real_value) = value.real_value {
        string.push_str(&format!("{:?}", real_value));
    };
    if let Some(ref string_value) = value.string_value {
        string.push_str(&string_value);
    };
    if let Some(ref struct_value) = value.struct_value {
        string.push_str(&format!("{:?}", struct_value));
    };
    string
}

fn one_row(values: &[Value]) -> impl Iterator<Item = String> + '_ {
    values.iter().map(|value| format_value(&value))
}

/// Return an iterator of iterators of strings
fn format_rows(
    result: &SqlStatementResult,
) -> impl Iterator<Item = impl Iterator<Item = String> + '_> {
    // This seems pretty crazed...
    result
        .result_frame
        .as_ref()
        .unwrap_or(&EMPTY_RESULT_FRAME)
        .records
        .as_ref()
        .map_or(&[][..], |x| &**x)
        .iter()
        .map(|record| one_row(record.values.as_ref().map_or(&[][..], |x| &**x)))
}

fn my_cluster(
    requested_db_cluster_identifier: &Option<String>,
    db_clusters: &[DBCluster],
) -> Option<DBCluster> {
    match requested_db_cluster_identifier {
        Some(requested_db_cluster_identifier) => {
            for db_cluster in db_clusters {
                if let Some(ref db_cluster_identifier) = db_cluster.db_cluster_identifier {
                    // Since this is an exact match, we assume there will only ever be one.
                    if requested_db_cluster_identifier == db_cluster_identifier {
                        return Some(db_cluster.to_owned());
                    }
                }
            }
            // NoMatchingCluster
            None
        }
        None => {
            // There is only one: go ahead and use it.
            match db_clusters.len() {
                1 => Some(db_clusters[0].to_owned()),
                0 => None, // NoClusters
                _ => None, // MultipleClusters
            }
        }
    }
}

fn my_secret(
    requested_db_cluster_resource_id: &str,
    requested_db_user_id: &Option<String>,
    secret_list: &[SecretListEntry],
) -> Option<SecretListEntry> {
    match requested_db_user_id {
        Some(requested_db_user_id) => {
            let the_name = "rds-db-credentials/".to_string()
                + requested_db_cluster_resource_id
                + "/"
                + requested_db_user_id;
            for secret_list_entry in secret_list {
                if let Some(ref name) = secret_list_entry.name {
                    if *name == the_name {
                        // Since this is an exact match, we assume there will only ever be one.
                        return Some(secret_list_entry.to_owned());
                    }
                }
            }
            None // NoMatchingSecret
        }
        None => {
            let name_starts_with =
                "rds-db-credentials/".to_string() + requested_db_cluster_resource_id + "/";
            let matches: Vec<&SecretListEntry> = secret_list
                .iter()
                .filter(|secret_list_entry| match secret_list_entry.name {
                    Some(ref name) => name.starts_with(&name_starts_with),
                    None => false,
                })
                .collect();
            match matches.len() {
                // There is only one: go ahead and use it.
                1 => Some(matches[0].to_owned()),
                0 => {
                    error!(
                        "No secrets found for database: {}",
                        requested_db_cluster_resource_id
                    );
                    None // NoSecrets
                }
                _ => {
                    let names: Vec<String> = matches
                        .iter()
                        .map(|entry| entry.name.as_ref().unwrap_or(&"".to_string()).to_owned())
                        .collect();
                    error!("Multiple secrets found {:?}", names);
                    None // MultipleSecrets
                }
            }
        }
    }
}

fn get_arns(
    runtime: &mut Runtime,
    region: &Region,
    requested_db_cluster_identifier: &Option<String>,
    requested_user_id: &Option<String>,
) -> Result<MyArns, Error> {
    let describe_db_clusters_message = DescribeDBClustersMessage {
        db_cluster_identifier: None,
        filters: None,
        include_shared: None,
        marker: None,
        max_records: None,
    };
    let list_secrets_request = ListSecretsRequest {
        max_results: None,
        next_token: None,
    };

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

    let (db_cluster_message, list_secrets_response) = runtime.block_on(fut1.join(fut2))?;
    info!("{:?}", db_cluster_message);
    info!("{:?}", list_secrets_response);
    let db_cluster = match db_cluster_message.db_clusters {
        Some(db_clusters) => my_cluster(requested_db_cluster_identifier, &db_clusters),
        None => None,
    };
    match db_cluster {
        Some(db_cluster) => {
            let secret_list_entry = match list_secrets_response.secret_list {
                Some(secret_list) => my_secret(
                    &db_cluster.db_cluster_resource_id.unwrap(),
                    &requested_user_id,
                    &secret_list,
                ),
                None => None,
            };
            match secret_list_entry {
                Some(secret_list_entry) => Ok(MyArns {
                    aws_secret_store_arn: secret_list_entry.arn.unwrap(),
                    db_cluster_or_instance_arn: db_cluster.db_cluster_arn.unwrap(),
                }),
                None => Err(Error::SecretNotFound {}),
            }
        }
        None => {
            // TODO Enhance this error message to list what was found
            Err(Error::DBNotFound {
                db_cluster_identifier: requested_db_cluster_identifier.clone().unwrap_or_default(),
            })
        }
    }
}

fn main() -> Result<(), ExitFailure> {
    let args = MyArgs::from_args();
    stderrlog::new()
        .quiet(args.quiet)
        .verbosity(args.verbose)
        .timestamp(args.ts.unwrap_or(stderrlog::Timestamp::Off))
        .init()?;
    env::set_var("AWS_PROFILE", args.profile);
    let region = Region::from_str(&args.region)?;
    let rds_data_client = RdsDataClient::new(region.clone());

    let mut runtime = Runtime::new()?;
    let my_arns = get_arns(&mut runtime, &region, &args.db_id, &args.user_id)?;

    let execute_sql_request = ExecuteSqlRequest {
        aws_secret_store_arn: my_arns.aws_secret_store_arn,
        database: None,
        db_cluster_or_instance_arn: my_arns.db_cluster_or_instance_arn,
        schema: None,
        sql_statements: args.query,
    };

    info!("{:?}", execute_sql_request);
    let fut = rds_data_client.execute_sql(execute_sql_request);
    // The result that comes back is fairly intense.
    // I don't know how to take it apart in a non-tedious way.
    let execute_sql_response = runtime.block_on(fut)?;
    info!("{:?}", execute_sql_response);
    let mut wtr = csv::Writer::from_writer(stdout());
    if let Some(results) = execute_sql_response.sql_statement_results {
        for result in &results {
            warn!(
                "number_of_records_updated: {}",
                result.number_of_records_updated.unwrap_or(-1)
            );
            wtr.write_record(format_header(result))?;
            for row in format_rows(result) {
                wtr.write_record(row)?;
            }
        }
    };
    Ok(())
}
