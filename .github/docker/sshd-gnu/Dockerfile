# An OpenSSH server on Debian: the `gnu` command profile of the SSH plugin
# (issue 34). The user `rick` logs in with the password `rickpass` or the
# public key in authorized_keys, which the CI job generates and mounts.
FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends openssh-server coreutils findutils \
 && rm -rf /var/lib/apt/lists/* \
 && mkdir -p /run/sshd \
 && useradd -m -s /bin/bash rick \
 && echo 'rick:rickpass' | chpasswd \
 && sed -i 's/^#\?PasswordAuthentication .*/PasswordAuthentication yes/' /etc/ssh/sshd_config \
 && sed -i 's/^#\?KbdInteractiveAuthentication .*/KbdInteractiveAuthentication yes/' /etc/ssh/sshd_config \
 && echo 'MaxSessions 10' >> /etc/ssh/sshd_config \
 && echo 'AllowTcpForwarding yes' >> /etc/ssh/sshd_config
COPY entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh
EXPOSE 22
ENTRYPOINT ["/entrypoint.sh"]
