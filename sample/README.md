# datem Lambda Sample

This sample deploys two datem Lambda functions to AWS using Terraform:

| Function | Trigger | Purpose |
|---|---|---|
| `datem-ingest` | SQS queue | Batch-ingests usage events written to the queue |
| `datem-billing` | EventBridge cron | Runs the billing engine on a schedule |

---

## Prerequisites

- AWS CLI authenticated (`aws sts get-caller-identity` works)
- Docker
- Terraform >= 1.6

---

## Deploy

### 1. Initialise Terraform

```bash
cd sample/infra
terraform init
```

### 2. Create a `terraform.tfvars` file

```hcl
bucket_name           = "my-datem-data"
region                = "us-east-1"
api_key               = "your-api-key"
stripe_key            = "sk_live_..."
stripe_webhook_secret = "whsec_..."
```

### 3. Apply (creates ECR repos and all other infra)

```bash
terraform apply
```

### 4. Build images and push to ECR

Run from the **workspace root**:

```bash
IMAGE_TAG=latest
REGION=us-east-1
ACCOUNT_ID=$(aws sts get-caller-identity --query Account --output text)

# Authenticate Docker to ECR
aws ecr get-login-password --region "$REGION" \
  | docker login --username AWS --password-stdin \
      "$ACCOUNT_ID.dkr.ecr.$REGION.amazonaws.com"

# Build and push both images
for FUNCTION in ingest billing; do
  ECR_REPO=$(cd sample/infra && terraform output -raw ${FUNCTION}_ecr_repository)

  docker build \
    -f lambda/Dockerfile \
    --build-arg FUNCTION=$FUNCTION \
    -t "$ECR_REPO:$IMAGE_TAG" \
    .

  docker push "$ECR_REPO:$IMAGE_TAG"
done
```

### 5. Apply again so Lambda picks up the pushed images

```bash
cd sample/infra
terraform apply
```

---

## Sending events

After deploy, get the SQS queue URL:

```bash
terraform output ingest_queue_url
```

Send events to the queue in the same JSON shape as the HTTP ingest API:

```bash
aws sqs send-message \
  --queue-url "$(terraform output -raw ingest_queue_url)" \
  --message-body '{
    "event_id":   "01HWXYZ123",
    "customer_id": "cust_abc",
    "metric":      "api_calls",
    "quantity":    1,
    "timestamp":   1718918400000000
  }'
```

---

## Updating

To deploy a new version:

```bash
# Rebuild and push images (repeat step 4 above)
# Then update Lambda:
cd sample/infra && terraform apply -var="image_tag=<new-tag>"
```

---

## Tear down

```bash
cd sample/infra && terraform destroy
```
