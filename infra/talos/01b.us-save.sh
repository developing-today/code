#!/usr/bin/env bash
tar czf - ./01b.us | base64 > 01b.us.tmp
sops --encrypt 01b.us.tmp > 01b.us.enc
rm 01b.us.tmp
