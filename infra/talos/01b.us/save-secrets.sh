#!/usr/bin/env bash
tar czf - ./secrets | base64 > secrets.tmp
sops --encrypt secrets.tmp > secrets.enc
rm secrets.tmp
