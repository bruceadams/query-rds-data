# Query AWS RDS Data from the command line

[![Build Status](https://api.cirrus-ci.com/github/bruceadams/query-rds-data.svg)](https://cirrus-ci.com/github/bruceadams/query-rds-data)
[![Contributor Covenant](https://img.shields.io/badge/Contributor%20Covenant-v1.4%20adopted-ff69b4.svg)](CODE_OF_CONDUCT.md)
[![Apache License](https://img.shields.io/github/license/bruceadams/query-rds-data?logo=apache)](LICENSE)
[![Github Release](https://img.shields.io/github/v/release/bruceadams/query-rds-data?logo=github)](https://github.com/bruceadams/query-rds-data/releases)
[![Crates.io](https://img.shields.io/crates/v/query-rds-data?logo=rust)](https://crates.io/crates/query-rds-data)

## Goal

## Installing

Prebuilt binaries for several platforms are available under
[Github releases](https://github.com/bruceadams/query-rds-data/releases).

On macOS, the prebuilt binary can be installed using [Homebrew](https://brew.sh).

```bash
$ brew tap bruceadams/utilities
$ brew install query-rds-data
```

If you have the [Rust tool chain](https://rustup.rs/) installed,
`cargo install query-rds-data` will work.

## Building

This project is written in Rust. **The** way to install Rust is from
[Rustup.rs](https://rustup.rs/). Once Rust is installed on your machine,
running `cargo  build` in  the root  of this checkout should _just work_
and produces a debug binary in `target/debug/query-rds-data`.

## Built-in help

```bash
$ cargo build  # The first build takes longer, with more output
    Finished dev [unoptimized + debuginfo] target(s) in 0.22s
$ target/debug/query-rds-data --help
query-rds-data 1.2.2

Query AWS RDS Data from the command line

USAGE:
    query-rds-data [FLAGS] [OPTIONS] <QUERY>

ARGS:
    <QUERY>    SQL query

FLAGS:
    -h, --help       Print help information
    -v, --verbose    Increase logging verbosity (-v, -vv, -vvv, etc)
    -V, --version    Print version information

OPTIONS:
    -c, --db-cluster-identifier <DB_ID>
            RDS cluster identifier [env: AWS_RDS_CLUSTER=]

    -d, --database <DATABASE>
            Database name [env: AWS_RDS_DATABASE=]

    -f, --format <FORMAT>
            Output format [default: csv] [possible values: csv, cooked, raw]

    -p, --aws-profile <PROFILE>
            AWS source profile to use. This name references an entry in
            ~/.aws/config [env: AWS_PROFILE=]

    -r, --aws-region <REGION>
            AWS region to target [env: AWS_DEFAULT_REGION=] [default: us-
            east-1]

    -u, --db-user-identifier <USER_ID>
            RDS user identifier (really the AWS secret identifier) [env:
            AWS_RDS_USER=]
```

## Error messages

I hope that the error messages from `query-rds-data` are helpful
for figuring out what went wrong and how to address the issue.

```bash
# No RDS instances exist
$ query-rds-data "select * from db1.names"
Error: No DBs found
$ query-rds-data "select * from db1.names" --db-cluster-identifier nope
Error: No DB matched "nope", available ids are []

# Single RDS instance exists
$ query-rds-data "select * from db1.names" --db-cluster-identifier nope
Error: No DB matched "nope", available ids are ["demo"]

# No credentials in AWS Secrets Manager
$ query-rds-data "select * from db1.names"
Error: No DB user secrets found
$ query-rds-data "select * from db1.names" --db-user-identifier fake
Error: No DB user matched "fake", available users are []

# Single secret exists for this database
$ query-rds-data "create database db1"
""
$ query-rds-data "create table db1.names (id int, name varchar(64))"
""
$ query-rds-data "insert into db1.names values (1,'Bruce')"
""
$ query-rds-data "select * from db1.names"
id,name
1,Bruce
$ query-rds-data "select * from information_schema.tables where table_schema='db1'"
TABLE_CATALOG,TABLE_SCHEMA,TABLE_NAME,TABLE_TYPE,ENGINE,VERSION,ROW_FORMAT,TABLE_ROWS,AVG_ROW_LENGTH,DATA_LENGTH,MAX_DATA_LENGTH,INDEX_LENGTH,DATA_FREE,AUTO_INCREMENT,CREATE_TIME,UPDATE_TIME,CHECK_TIME,TABLE_COLLATION,CHECKSUM,CREATE_OPTIONS,TABLE_COMMENT
def,db1,names,BASE TABLE,InnoDB,10,Compact,0,0,16384,0,0,0,NULL,NULL,NULL,NULL,latin1_swedish_ci,NULL,,

# Explicit cluster and user names can be used
$ query-rds-data "select * from db1.names" --db-cluster-identifier demo
id,name
1,Bruce
$ query-rds-data "select * from db1.names" --db-user-identifier admin
id,name
1,Bruce
$ query-rds-data "select * from db1.names" --db-cluster-identifier demo --db-user-identifier admin
id,name
1,Bruce

# Names that are not found list what is available to be selected
$ query-rds-data "select * from db1.names" --db-cluster-identifier nope
Error: No DB matched "nope", available ids are ["demo"]
$ query-rds-data "select * from db1.names" --db-user-identifier fake
Error: No DB user matched "fake", available users are ["admin"]

# If there are multiple clusters or users available, you must select one
$ query-rds-data "select * from db1.names"
Error: Multiple DBs found, please specify one of ["demo", "empty"]
$ query-rds-data "select * from db1.names" --db-cluster-identifier demo
Error: Multiple DB users found, please specify one of ["admin", "read_only"]
$ query-rds-data "select * from db1.names"  --db-cluster-identifier demo --db-user-identifier read_only
id,name
1,Bruce
```
