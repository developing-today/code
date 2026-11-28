#!/usr/bin/env bash

pg_ctl -D .tmp/db -l logfile -o "--unix_socket_directories='$PWD'" start
