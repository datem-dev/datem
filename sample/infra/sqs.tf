resource "aws_sqs_queue" "ingest_dlq" {
  name                      = "datem-ingest-dlq"
  message_retention_seconds = 1209600 # 14 days
}

resource "aws_sqs_queue" "ingest" {
  name                       = "datem-ingest"
  visibility_timeout_seconds = 300 # must be >= Lambda timeout
  message_retention_seconds  = 86400

  redrive_policy = jsonencode({
    deadLetterTargetArn = aws_sqs_queue.ingest_dlq.arn
    maxReceiveCount     = 3
  })
}

# Allow anyone with the queue URL to send messages (callers use their own IAM).
# Tighten this to specific IAM principals in production.
resource "aws_sqs_queue_policy" "ingest" {
  queue_url = aws_sqs_queue.ingest.id
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect    = "Allow"
      Principal = { AWS = "arn:aws:iam::${data.aws_caller_identity.current.account_id}:root" }
      Action    = "sqs:SendMessage"
      Resource  = aws_sqs_queue.ingest.arn
    }]
  })
}

resource "aws_lambda_event_source_mapping" "ingest" {
  event_source_arn                   = aws_sqs_queue.ingest.arn
  function_name                      = aws_lambda_function.ingest.arn
  batch_size                         = var.ingest_batch_size
  maximum_batching_window_in_seconds = 5
  function_response_types            = ["ReportBatchItemFailures"]
}
