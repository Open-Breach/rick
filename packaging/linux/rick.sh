#!/bin/sh
# /usr/bin/rick of the deb and the rpm: Rick lives in /usr/lib/rick/, with
# its libraries and plugins beside the binary as everywhere, and this starts
# it from there. A script and not a link, since one of the two packaging
# tools has no links.
exec /usr/lib/rick/rick "$@"
