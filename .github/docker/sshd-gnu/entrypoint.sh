#!/bin/sh
# Installs the public key the job mounted at /keys/id.pub, if any, for the
# user rick, then runs sshd in the foreground.
set -eu
if [ -f /keys/id.pub ]; then
  mkdir -p /home/rick/.ssh
  cat /keys/*.pub > /home/rick/.ssh/authorized_keys
  chown -R rick:rick /home/rick/.ssh
  chmod 700 /home/rick/.ssh
  chmod 600 /home/rick/.ssh/authorized_keys
fi
ssh-keygen -A
exec /usr/sbin/sshd -D -e
