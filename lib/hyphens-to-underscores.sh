#!/usr/bin/env bash

find . -type f -name "*-*" -exec sh -c 'mv "$0" "${0//-/_}"' {} \;

