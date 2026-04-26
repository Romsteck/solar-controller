#!/usr/bin/env bash
set -e
cd "$(dirname "$0")/.."
python deploy/deploy.py "$@"
