# Query AWS RDS Data from the command line
[![Build Status](https://img.shields.io/travis/com/bruceadams/query-rds-data?logo=travis)](https://travis-ci.com/bruceadams/query-rds-data)
[![Contributor Covenant](https://img.shields.io/badge/Contributor%20Covenant-v1.4%20adopted-ff69b4.svg)](CODE_OF_CONDUCT.md)
[![Apache License](https://img.shields.io/github/license/bruceadams/query-rds-data?logo=apache)](LICENSE)
[![Github Release](https://img.shields.io/github/v/release/bruceadams/query-rds-data?logo=github)](https://github.com/bruceadams/query-rds-data/releases)
[![Crates.io](https://img.shields.io/crates/v/query-rds-data?logo=rust)](https://crates.io/crates/query-rds-data)

## Goal

## Installing

Prebuilt binaries for some major platforms are available under
[Github releases](https://github.com/bruceadams/query-rds-data/releases).

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
query-rds-data 0.5.0
Query an Amazon RDS database

USAGE:
    query-rds-data [FLAGS] [OPTIONS] <query> --aws-profile <profile>

FLAGS:
    -h, --help       Prints help information
    -q, --quiet      Silence all output
    -V, --version    Prints version information
    -v, --verbose    Verbose mode (-v, -vv, -vvv, etc)

OPTIONS:
    -c, --db-cluster-identifier <db-id>    RDS database identifier.
    -p, --aws-profile <profile>
            AWS source profile to use. This name references an entry
            in ~/.aws/credentials [env: AWS_PROFILE=]
    -r, --aws-region <region>
            AWS region to target. [env: AWS_DEFAULT_REGION=]
            [default: us-east-1]
    -t, --timestamp <ts>
            Timestamp (sec, ms, ns, none)

    -u, --db-user-identifier <user-id>
            RDS user identifier (really the AWS secret identifier).


ARGS:
    <query>    SQL query.
```
