# An OpenSSH server on Alpine: the BusyBox userland behind the `busybox`
# command profile of the SSH plugin (issue 34). Same user and credentials
# as the Debian image.
FROM alpine:3
RUN apk add --no-cache openssh bash \
 && mkdir -p /run/sshd \
 && adduser -D -s /bin/sh rick \
 && echo 'rick:rickpass' | chpasswd \
 && sed -i 's/^#\?PasswordAuthentication .*/PasswordAuthentication yes/' /etc/ssh/sshd_config \
 && sed -i 's/^#\?KbdInteractiveAuthentication .*/KbdInteractiveAuthentication yes/' /etc/ssh/sshd_config \
 && echo 'MaxSessions 10' >> /etc/ssh/sshd_config \
 && echo 'AllowTcpForwarding yes' >> /etc/ssh/sshd_config
COPY entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh
EXPOSE 22
ENTRYPOINT ["/entrypoint.sh"]
