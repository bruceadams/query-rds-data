#!/bin/bash

set -euxo pipefail

BINARY=query-rds-data

if [[ "$TRAVIS_OS_NAME" == "windows" ]]; then
    cargo build --release
    cp -fp "target/release/$BINARY.exe" "target"
elif [[ "$TRAVIS_OS_NAME" == "osx" ]]; then
    cargo build --release
    # I'm picky about naming: "macos"
    cp -fp "target/release/$BINARY" "target/$BINARY.macos"
else
    cargo build --release
    cp -fp "target/release/$BINARY" "target/$BINARY.$TRAVIS_OS_NAME"
fi
