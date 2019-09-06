use exitfailure::ExitFailure;
use failure::{format_err, Error};
use futures::prelude::*;
use log::{info, warn};
use rusoto_core::region::Region;
use rusoto_rds::{DBCluster, DescribeDBClustersMessage, Rds, RdsClient};
use rusoto_rds_data::{ExecuteSqlRequest, RdsData, RdsDataClient, SqlStatementResult, Value};
use rusoto_secretsmanager::{
    ListSecretsRequest, SecretListEntry, SecretsManager, SecretsManagerClient,
};
use std::env;
use std::str::FromStr;
use structopt::{clap::AppSettings::ColoredHelp, StructOpt};
use tokio::runtime::Runtime;

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
    #[structopt(long = "db-cluster-identifier", short = "i")]
    db_id: Option<String>,

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

struct MyArns {
    aws_secret_store_arn: String,
    db_cluster_or_instance_arn: String,
}

/// Print out the name for each column, comma separated.
/// This will misbehave for weird output data.
fn print_header(result: &SqlStatementResult) {
    let mut first = true;
    // This seems pretty crazed...
    if let Some(ref result_frame) = result.result_frame {
        if let Some(ref result_set_metadata) = result_frame.result_set_metadata {
            if let Some(ref column_metadata) = result_set_metadata.column_metadata {
                for column in column_metadata {
                    if first {
                        first = false;
                    } else {
                        print!(",");
                    }
                    if let Some(ref label) = column.label {
                        print!("{}", label);
                    } else if let Some(ref name) = column.name {
                        print!("{}", name);
                    } else {
                        print!("?");
                    }
                }
            }
        }
    }
    println!();
}

fn print_value(value: &Value) {
    if let Some(ref array_values) = value.array_values {
        print!("{:?}", array_values);
    };
    if let Some(ref big_int_value) = value.big_int_value {
        print!("{:?}", big_int_value);
    };
    if let Some(ref bit_value) = value.bit_value {
        print!("{:?}", bit_value);
    };
    if let Some(ref blob_value) = value.blob_value {
        print!("{:?}", blob_value);
    };
    if let Some(ref double_value) = value.double_value {
        print!("{:?}", double_value);
    };
    if let Some(ref int_value) = value.int_value {
        print!("{:?}", int_value);
    };
    if let Some(ref _is_null) = value.is_null {
        print!("NULL");
    };
    if let Some(ref real_value) = value.real_value {
        print!("{:?}", real_value);
    };
    if let Some(ref string_value) = value.string_value {
        print!("{:?}", string_value);
    };
    if let Some(ref struct_value) = value.struct_value {
        print!("{:?}", struct_value);
    };
}

/// Print out the name for each column, comma separated.
/// This will misbehave for weird output data.
fn print_rows(result: &SqlStatementResult) {
    // This seems pretty crazed...
    if let Some(ref result_frame) = result.result_frame {
        if let Some(ref records) = result_frame.records {
            for record in records {
                let mut first = true;
                if let Some(ref values) = record.values {
                    for value in values {
                        if first {
                            first = false;
                        } else {
                            print!(",");
                        }
                        print_value(&value);
                    }
                }
                println!();
            }
        }
    }
}

fn my_cluster(
    requested_db_cluster_identifier: &Option<String>,
    db_clusters: &[DBCluster],
) -> Option<DBCluster> {
    match requested_db_cluster_identifier {
        Some(requested_db_cluster_identifier) => {
            for db_cluster in db_clusters {
                if let Some(ref db_cluster_identifier) = db_cluster.db_cluster_identifier {
                    if requested_db_cluster_identifier == db_cluster_identifier {
                        return Some(db_cluster.to_owned());
                    }
                }
            }
        }
        None => {
            if db_clusters.len() == 1 {
                return Some(db_clusters[0].to_owned());
            }
        }
    }
    None
}

fn my_secret(
    requested_db_cluster_resource_id: &str,
    secret_list: &[SecretListEntry],
) -> Option<SecretListEntry> {
    let name_starts_with =
        "rds-db-credentials/".to_string() + requested_db_cluster_resource_id + "/";
    let mut the_one = None;
    // FIXME: pick the correct id, not just the last one
    for secret_list_entry in secret_list {
        if let Some(ref name) = secret_list_entry.name {
            if name.starts_with(&name_starts_with) {
                the_one = Some(secret_list_entry.to_owned());
            }
        }
    }
    the_one
}

fn get_arns(
    runtime: &mut Runtime,
    region: &Region,
    requested_db_cluster_identifier: &Option<String>,
) -> Result<MyArns, Error> {
    let describe_db_clusters_message = DescribeDBClustersMessage {
        db_cluster_identifier: None,
        filters: None,
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
        .map_err(Error::from);
    let fut2 = secrets_manager_client
        .list_secrets(list_secrets_request)
        .map_err(Error::from);

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
                Some(secret_list) => {
                    my_secret(&db_cluster.db_cluster_resource_id.unwrap(), &secret_list)
                }
                None => None,
            };
            match secret_list_entry {
                Some(secret_list_entry) => Ok(MyArns {
                    aws_secret_store_arn: secret_list_entry.arn.unwrap(),
                    db_cluster_or_instance_arn: db_cluster.db_cluster_arn.unwrap(),
                }),
                None => Err(format_err!("secret not found")),
            }
        }
        None => {
            Err(format_err!(
                // TODO Enhance this error message to list what was found
                "db_cluster_identifier={:?} not found",
                requested_db_cluster_identifier
            ))
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
    let my_arns = get_arns(&mut runtime, &region, &args.db_id)?;

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
    if let Some(results) = execute_sql_response.sql_statement_results {
        for result in &results {
            warn!("number_of_records_updated: {}", result.number_of_records_updated.unwrap_or(-1));
            print_header(result);
            print_rows(result);
        }
    };
    Ok(())
}
