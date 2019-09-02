use quicli::prelude::*;
use rusoto_core::region::Region;
use rusoto_rds_data::{ExecuteSqlRequest, RdsData, RdsDataClient, SqlStatementResult, Value};
use std::env;
use structopt::StructOpt;

/// Query an Amazon RDS database
#[derive(Debug, StructOpt)]
struct MyArgs {
    /// AWS source profile to use. This name references an entry in ~/.aws/credentials
    #[structopt(
        env = "AWS_PROFILE",
        long = "aws-profile",
        short = "p"
    )]
    aws_profile: String,

    /// AWS region to target.
    #[structopt(
        default_value = "us-east-1",
        env = "AWS_DEFAULT_REGION",
        long = "aws-region",
        short = "r"
    )]
    aws_region: String,

    /// SQL query.
    query: String,
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
    println!("");
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
                println!("");
            }
        }
    }
}

fn main() -> CliResult {
    let args = MyArgs::from_args();
    env::set_var("AWS_PROFILE", args.aws_profile);
    env::set_var("AWS_DEFAULT_REGION", args.aws_region);

    let rds_data_client = RdsDataClient::new(Region::default());

    // In theory, we can grab the secret from the AWS secret manager.
    // Similarly, we should be able to dig out the instance arn from RDS.
    let input = ExecuteSqlRequest {
        aws_secret_store_arn: "".to_string(),
        database: None,
        db_cluster_or_instance_arn: "".to_string(),
        schema: None,
        sql_statements: args.query,
    };

    let fut = rds_data_client.execute_sql(input);
    // The result that comes back is fairly intense.
    // I don't know how to take it apart in a non-tedious way.
    let response = fut.sync()?;
    if let Some(results) = response.sql_statement_results {
        for ref result in results {
            print_header(result);
            print_rows(result);
        }
    };
    Ok(())
}
