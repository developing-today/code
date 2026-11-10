#!/bin/sh
# Compression of the seed-1 dumps written by the harness (out/). Prints raw and compressed sizes.
cd "$(dirname "$0")/../out" || exit 1
for f in stock-s1-welcome-k1.bin stock-s1-welcome-k1000.bin both-s1-welcome-k1.bin both-s1-welcome-k1000.bin \
         transcoded-outer-s1-welcome-k1000.bin stock-s1-update-dm-k1000.bin both-s1-update-dm-k1000.bin \
         stock-s1-readd-k1000.bin both-s1-readd-k1000.bin; do
  raw=$(wc -c < "$f")
  z=$(zstd -19 -q -c "$f" | wc -c); x=$(xz -9e -c "$f" | wc -c); g=$(gzip -9 -c "$f" | wc -c)
  b=$(bzip2 -9 -c "$f" | wc -c); q=$(brotli -q 11 -c "$f" | wc -c)
  echo "$f raw=$raw zstd19=$z xz9e=$x gzip9=$g bzip2_9=$b brotli11=$q"
done
