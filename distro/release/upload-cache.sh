#!/usr/bin/env bash
# upload-cache.sh DIR URL: upload a binary cache directory to an Azure Blob container
# path (URL, without a trailing slash), with the shared access signature in
# AZURE_STORAGE_SAS. Files that are there already are left alone: a cache only grows,
# and a store path's content never changes. Needs curl and nothing else.
set -euo pipefail
dir=$1 url=$2
sas=${AZURE_STORAGE_SAS:?a shared access signature that may create blobs}
sas=${sas#\?}
put() {
  file=$1 name=${1#"$dir"/}
  if curl -sf -o /dev/null -I "$url/$name"; then echo "have $name"; return; fi
  curl -sf --retry 3 -X PUT -H "x-ms-blob-type: BlockBlob" -H "x-ms-version: 2021-08-06" \
    -H "Content-Type: application/octet-stream" \
    --data-binary "@$file" "$url/$name?$sas" -o /dev/null || { echo "FAILED $name" >&2; return 1; }
  echo "put  $name"
}
export -f put; export dir url sas
find "$dir" -type f -print0 | xargs -0 -P 8 -n 1 bash -c 'put "$0"' | sort | uniq -c -w 4 | awk '{print $1, $2}'
curl -sf "$url/nix-cache-info" | head -3
