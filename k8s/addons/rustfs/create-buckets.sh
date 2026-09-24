#!/bin/sh
# initContainer helper (curlimages/curl): create $BUCKETS on RustFS via SigV4 PUT, retrying until S3 is up and
# every bucket is really listed (an early PUT during RustFS start-up can return 200 and be lost).
# Needs env RUSTFS_ACCESS_KEY / RUSTFS_SECRET_KEY / BUCKETS (space separated). 409 = already owned = fine.
EMPTY=e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
s3() { curl -s "$@" --user "$RUSTFS_ACCESS_KEY:$RUSTFS_SECRET_KEY" --aws-sigv4 aws:amz:us-east-1:s3 -H "x-amz-content-sha256: $EMPTY"; }
until curl -sf http://rustfs:9000/health >/dev/null; do sleep 2; done
while :; do
  ok=1
  for b in $BUCKETS; do
    c=$(s3 -o /dev/null -w '%{http_code}' -X PUT "http://rustfs:9000/$b")
    case "$c" in 200|409) ;; *) ok=""; echo "bucket $b: HTTP $c" ;; esac
  done
  if [ -n "$ok" ]; then
    list=$(s3 http://rustfs:9000/)
    for b in $BUCKETS; do case "$list" in *"<Name>$b</Name>"*) ;; *) ok="" ;; esac; done
  fi
  [ -n "$ok" ] && break
  sleep 3
done
echo "buckets ready: $BUCKETS"
