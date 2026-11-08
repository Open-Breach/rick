#!/usr/bin/env bash
# The containers the provider plugins' round trips run against (issue 21):
# MinIO for S3, Azurite for Azure Blob, fake-gcs-server for Google Cloud
# Storage and an sshd (atmoz/sftp) for SFTP, started with `docker run` on
# a Linux runner and waited for, then named to the tests through the
# RICK_* variables in GITHUB_ENV. RICK_REQUIRE_CONTAINERS makes a test
# that would skip for a missing variable fail instead, so a container that
# did not come up is a red run, not a quiet skip. The tests clean up what
# they create; the containers die with the job.
#
# Issue 34 adds the SSH plugin's hosts: an OpenSSH server on Debian (the
# `gnu` profile) and one on Alpine (`busybox`), both published on
# localhost, and a bastion network where `rick-ssh-hop` and
# `rick-ssh-target` are reachable only through `rick-ssh-bastion`, so the
# jump-host tests cross one and two hops. The job's key pair (plain and
# passphrase-protected) is authorized on every host, and an ssh-agent holds
# the plain key for the agent test.
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

# The SSH hosts (issue 34): two images built from .github/docker, the key
# pair the tests authenticate with, and the bastion network.
keys="$RUNNER_TEMP/rick-ssh-keys"
mkdir -p "$keys"
ssh-keygen -q -t ed25519 -N '' -f "$keys/id_ed25519"
ssh-keygen -q -t ed25519 -N 'rickphrase' -f "$keys/id_locked"
cp "$keys/id_ed25519.pub" "$keys/id.pub"
cp "$keys/id_locked.pub" "$keys/locked.pub"
chmod 644 "$keys"/*.pub
docker build -q -t rick-sshd-gnu .github/docker/sshd-gnu
docker build -q -t rick-sshd-busybox .github/docker/sshd-busybox
docker network create rick-ssh
docker run -d --name rick-ssh-gnu --network rick-ssh -p 2223:22 -v "$keys:/keys:ro" rick-sshd-gnu
docker run -d --name rick-ssh-busybox -p 2224:22 -v "$keys:/keys:ro" rick-sshd-busybox
docker run -d --name rick-ssh-bastion --network rick-ssh -p 2225:22 -v "$keys:/keys:ro" rick-sshd-gnu
docker run -d --name rick-ssh-hop --network rick-ssh -v "$keys:/keys:ro" rick-sshd-gnu
docker run -d --name rick-ssh-target --network rick-ssh -v "$keys:/keys:ro" rick-sshd-busybox
for port in 2223 2224 2225; do
  tries=60
  until (exec 3<>/dev/tcp/127.0.0.1/$port) 2>/dev/null; do
    tries=$((tries - 1))
    if [ "$tries" -le 0 ]; then
      echo "sshd on $port did not come up"
      docker ps -a
      exit 1
    fi
    sleep 1
  done
  echo "sshd on $port is up"
done
# An agent holding the plain key, for the agent authentication test.
eval "$(ssh-agent -s)" > /dev/null
ssh-add -q "$keys/id_ed25519"
echo "SSH_AUTH_SOCK=$SSH_AUTH_SOCK" >> "$GITHUB_ENV"

{
  echo "RICK_SSH_GNU_PORT=2223"
  echo "RICK_SSH_BUSYBOX_PORT=2224"
  echo "RICK_SSH_BASTION_PORT=2225"
  echo "RICK_SSH_HOP=rick-ssh-hop"
  echo "RICK_SSH_TARGET=rick-ssh-target"
  echo "RICK_SSH_USER=rick"
  echo "RICK_SSH_PASSWORD=rickpass"
  echo "RICK_SSH_KEY_FILE=$keys/id_ed25519"
  echo "RICK_SSH_LOCKED_KEY_FILE=$keys/id_locked"
  echo "RICK_SSH_PASSPHRASE=rickphrase"
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
