#!/usr/bin/env bash

initdb -D .tmp/db/postgres
pg_ctl -D .tmp/db/postgres -l logfile -o "--unix_socket_directories='$PWD/.tmp/db/unix_socket_directories'" start
createdb -h "$(pwd)/.tmp/db/unix_socket_directories" db
