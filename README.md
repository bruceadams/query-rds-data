# Query AWS RDS Data from the command line
[![Build Status](https://travis-ci.com/bruceadams/query-rds-data.svg?branch=master)](https://travis-ci.com/bruceadams/query-rds-data)



```bash
$ cargo build
...
$ target/debug/query-rds-data --help
query-rds-data 0.3.3
Query an Amazon RDS database

USAGE:
    query-rds-data [FLAGS] [OPTIONS] <query> --aws-profile <profile>

FLAGS:
    -h, --help       Prints help information
    -q, --quiet      Silence all output
    -V, --version    Prints version information
    -v, --verbose    Verbose mode (-v, -vv, -vvv, etc)

OPTIONS:
    -i, --db-cluster-identifier <db-id>
            RDS database identifier.

    -p, --aws-profile <profile>
            AWS source profile to use. This name references
            an entry in ~/.aws/credentials [env:
            AWS_PROFILE=]
    -r, --aws-region <region>
            AWS region to target. [env: AWS_DEFAULT_REGION=]
            [default: us-east-1]
    -t, --timestamp <ts>
            Timestamp (sec, ms, ns, none)


ARGS:
    <query>    SQL query.
```
