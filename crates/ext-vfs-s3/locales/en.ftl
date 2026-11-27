# ext-vfs-s3 — English, the fallback language.

s3-s3 = S3
s3-endpoint-empty-for-aws = Endpoint
s3-region = Region
s3-access-key = Access key
s3-secret-key = Secret key
s3-buckets = Buckets
s3-buckets-note = Names separated by commas. Empty lists all buckets.
s3-path-style-addressing = Path-style addressing
s3-files-live-in-a-bucket = Files live inside a bucket
s3-buckets-come-from-the-console = Buckets are created in the provider's console
s3-accounts-come-from-the-side-panel = Accounts are added in the side panel

# What a refusal says (specs/s3-connection.md 1.2); $code is the service's
# own error code.
s3-error-list-buckets-denied = This key is not allowed to list buckets. Add the bucket name or names in the connection's Buckets field. ({ $code })
s3-error-access-denied = Access to this bucket or object was denied. ({ $code })
s3-error-invalid-access-key = The access key is not recognised. ({ $code })
s3-error-signature = The signature was rejected, usually a wrong secret key. ({ $code })
s3-error-no-such-bucket = The bucket does not exist; check the name. ({ $code })
s3-error-other = { $code } ({ $status }): { $message }
s3-error-http = HTTP { $status }: { $message }

# The provider recognised from the endpoint, shown by the field
# (specs/s3-connection.md 1.3).
s3-provider-aws = Amazon S3
s3-provider-r2 = Cloudflare R2
s3-provider-b2 = Backblaze B2
s3-provider-wasabi = Wasabi
s3-provider-spaces = DigitalOcean Spaces
s3-provider-scaleway = Scaleway
s3-provider-ovh = OVHcloud
s3-provider-gcs = Google Cloud Storage (S3-compatible)

# The connection test (specs/s3-connection.md Part 3): its checks, and
# what a check says when it did not pass.
s3-test-reachable = The host is reachable
s3-test-credentials = The credentials are accepted
s3-test-list-buckets = The key may list buckets
s3-test-buckets = Each bucket named in Buckets can be opened
s3-test-bucket = The bucket { $name } can be opened
s3-test-timeout = No answer within { $seconds } seconds.
s3-test-list-denied-info = This key may not list buckets. The connection shows the buckets named in the Buckets field. ({ $code })
s3-test-no-secret = The secret key is missing.
