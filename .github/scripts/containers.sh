#!/usr/bin/env bash
# The containers the provider plugins' round trips run against (issue 21):
# MinIO for S3, Azurite for Azure Blob, fake-gcs-server for Google Cloud
# Storage and an sshd (atmoz/sftp) for SFTP, started with `docker run` on
# a Linux runner and waited for, then named to the tests through the
# RICK_* variables in GITHUB_ENV. RICK_REQUIRE_CONTAINERS makes a test
# that would skip for a missing variable fail instead, so a container that
# did not come up is a red run, not a quiet skip. The tests clean up what
# they create; the containers die with the job.
set -euo pipefail

# MinIO's community edition is source only since 2025 and Docker Hub's
# minio/minio is gone; Bitnami's frozen build of the last release is the
# MinIO that still pulls, pinned since the legacy repository never moves.
docker run -d --name rick-minio -p 9000:9000 \
  -e MINIO_ROOT_USER=minioadmin -e MINIO_ROOT_PASSWORD=minioadmin \
  bitnamilegacy/minio:2025.7.23-debian-12-r5
docker run -d --name rick-azurite -p 10000:10000 \
  mcr.microsoft.com/azure-storage/azurite azurite-blob --blobHost 0.0.0.0 --skipApiVersionCheck
docker run -d --name rick-gcs -p 4443:4443 \
  fsouza/fake-gcs-server -scheme http -port 4443 -public-host 127.0.0.1:4443
# The user `rick` with a password, chrooted to its home, where only
# `upload` is writable: the folder the connection opens at.
docker run -d --name rick-sftp -p 2222:22 atmoz/sftp:alpine rick:rickpass:1001::upload

wait_for() {
  local name="$1" url="$2" tries=60
  until curl -sS -o /dev/null "$url" 2>/dev/null; do
    tries=$((tries - 1))
    if [ "$tries" -le 0 ]; then
      echo "$name did not come up"
      docker logs "rick-$name" || true
      exit 1
    fi
    sleep 1
  done
  echo "$name is up"
}
wait_for minio http://127.0.0.1:9000/minio/health/live
wait_for azurite "http://127.0.0.1:10000/devstoreaccount1?comp=list"
wait_for gcs http://127.0.0.1:4443/storage/v1/b
tries=60
until (exec 3<>/dev/tcp/127.0.0.1/2222) 2>/dev/null; do
  tries=$((tries - 1))
  if [ "$tries" -le 0 ]; then
    echo "sftp did not come up"
    docker logs rick-sftp || true
    exit 1
  fi
  sleep 1
done
echo "sftp is up"

{
  echo "RICK_S3_ENDPOINT=http://127.0.0.1:9000"
  echo "RICK_S3_ACCESS_KEY=minioadmin"
  echo "RICK_S3_SECRET_KEY=minioadmin"
  echo "RICK_AZURE_ENDPOINT=http://127.0.0.1:10000"
  echo "RICK_AZURE_ACCOUNT=devstoreaccount1"
  echo "RICK_AZURE_KEY=Eby8vdM02xNOcqFlqUwJPLlmEtlCDXJ1OUzFT50uSRZ6IFsuFq2UVErCz4I6tq/K1SZFPTOtr/KBHBeksoGMGw=="
  echo "RICK_GCS_ENDPOINT=http://127.0.0.1:4443"
  echo "RICK_GCS_PROJECT=rick-test"
  echo "RICK_SFTP_HOST=127.0.0.1"
  echo "RICK_SFTP_PORT=2222"
  echo "RICK_SFTP_USER=rick"
  echo "RICK_SFTP_PASSWORD=rickpass"
  echo "RICK_SFTP_FOLDER=upload"
  echo "RICK_REQUIRE_CONTAINERS=1"
} >> "$GITHUB_ENV"
