# Query AWS RDS Data from the command line



```bash
$ cargo build
...
$ target/debug/query-rds-data --help
query-rds-data 0.1.0
Query an Amazon RDS database

USAGE:
    query-rds-data [OPTIONS] <query> --aws-profile <aws-profile>

FLAGS:
    -h, --help       Prints help information
    -V, --version    Prints version information

OPTIONS:
    -p, --aws-profile <aws-profile>
            AWS source profile to use. This name references an entry in
            ~/.aws/credentials [env: AWS_PROFILE=sandbox]
    -r, --aws-region <aws-region>
            AWS region to target. [env: AWS_DEFAULT_REGION=]  [default:
            us-east-1]
    -i, --db-cluster-identifier <db-cluster-identifier>
            RDS database identifier, for example: indi-primary.


ARGS:
    <query>    SQL query.
```
