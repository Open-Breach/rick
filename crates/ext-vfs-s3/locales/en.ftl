# ext-vfs-s3 — English, the fallback language.

s3-s3 = S3
s3-endpoint-empty-for-aws = Endpoint (empty for AWS)
s3-region = Region
s3-access-key = Access key
s3-secret-key = Secret key
s3-buckets = Buckets (comma-separated)
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
