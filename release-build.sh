#!/bin/bash

set -euxo pipefail

BINARY=query-rds-data

if [[ "$TRAVIS_OS_NAME" == "linux" ]]; then
    docker run -e NOSTRIP=1 -v "$PWD":/build fredrikfornwall/rust-static-builder
    cp -fp "target/x86_64-unknown-linux-musl/release/$BINARY" "target/$BINARY.linux"
elif [[ "$TRAVIS_OS_NAME" == "osx" ]]; then
    cargo build --release
    # I'm picky about naming: "macos"
    cp -fp "target/release/$BINARY" "target/$BINARY.macos"
elif [[ "$TRAVIS_OS_NAME" == "windows" ]]; then
    cargo build --release
    cp -fp "target/release/$BINARY.exe" "target"
else
    cargo build --release
    cp -fp "target/release/$BINARY" "target/$BINARY.$TRAVIS_OS_NAME"
fi
