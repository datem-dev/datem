locals {
  common_env = {
    DATEM_S3_BUCKET  = var.bucket_name
    DATEM_S3_REGION  = var.region
    DATEM_S3_PREFIX  = var.s3_prefix
    DATEM_DATA_DIR   = "/tmp/datem"
    DATEM_API_KEY    = var.api_key
    DATEM_STRIPE_KEY = var.stripe_key
    RUST_LOG         = "datem_lambda=info"
  }
}

resource "aws_lambda_function" "ingest" {
  function_name = "datem-ingest"
  role          = aws_iam_role.lambda.arn
  package_type  = "Image"
  image_uri     = "${aws_ecr_repository.ingest.repository_url}:${var.image_tag}"
  timeout       = 300
  memory_size   = 512

  ephemeral_storage {
    size = 1024
  }

  environment {
    variables = local.common_env
  }

  # Reserved concurrency = 1 prevents concurrent writes to the same Tonbo
  # manifest on S3. Throughput scales via batch_size, not parallelism.
  reserved_concurrent_executions = 1

  depends_on = [aws_iam_role_policy.lambda]
}

resource "aws_lambda_function" "billing" {
  function_name = "datem-billing"
  role          = aws_iam_role.lambda.arn
  package_type  = "Image"
  image_uri     = "${aws_ecr_repository.billing.repository_url}:${var.image_tag}"
  timeout       = 900
  memory_size   = 1024

  ephemeral_storage {
    size = 2048
  }

  environment {
    variables = merge(local.common_env, {
      DATEM_STRIPE_KEY = var.stripe_key
    })
  }

  reserved_concurrent_executions = 1

  depends_on = [aws_iam_role_policy.lambda]
}
