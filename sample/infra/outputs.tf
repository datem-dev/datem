output "ingest_queue_url" {
  description = "SQS queue URL — send events here"
  value       = aws_sqs_queue.ingest.url
}

output "ingest_dlq_url" {
  description = "Dead-letter queue URL for failed ingest messages"
  value       = aws_sqs_queue.ingest_dlq.url
}

output "ingest_ecr_repository" {
  description = "ECR repository URL for the ingest image"
  value       = aws_ecr_repository.ingest.repository_url
}

output "billing_ecr_repository" {
  description = "ECR repository URL for the billing image"
  value       = aws_ecr_repository.billing.repository_url
}

output "data_bucket" {
  description = "S3 bucket storing Parquet data"
  value       = aws_s3_bucket.data.bucket
}
