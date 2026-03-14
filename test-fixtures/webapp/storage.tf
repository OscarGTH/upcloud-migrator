# ─────────────────────────────────────────────────────────────────────────────
# Storage layer
# S3 buckets, EFS filesystem
# (EBS volumes are in compute.tf, co-located with their instances)
# ─────────────────────────────────────────────────────────────────────────────

# Static assets bucket  →  upcloud_managed_object_storage + _bucket
resource "aws_s3_bucket" "assets" {
  bucket = "webapp-assets-prod"

  tags = {
    Name    = "webapp-assets"
    Project = "webapp"
  }
}

# User uploads bucket
resource "aws_s3_bucket" "uploads" {
  bucket = "webapp-uploads-prod"

  tags = {
    Name    = "webapp-uploads"
    Project = "webapp"
  }
}

# Terraform state bucket (no bucket attribute — name falls back to resource name)
resource "aws_s3_bucket" "tf_state" {
  tags = {
    Name    = "webapp-tf-state"
    Project = "webapp"
  }
}

# Bucket policy  →  Partial (UpCloud uses access keys and UI policies)
resource "aws_s3_bucket_policy" "assets_policy" {
  bucket = aws_s3_bucket.assets.id
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect    = "Allow"
      Principal = "*"
      Action    = "s3:GetObject"
      Resource  = "${aws_s3_bucket.assets.arn}/*"
    }]
  })
}

resource "aws_s3_bucket_acl" "uploads_acl" {
  bucket = aws_s3_bucket.uploads.id
  acl    = "private"
}

# Shared filesystem for app servers  →  upcloud_file_storage
resource "aws_efs_file_system" "shared" {
  creation_token = "webapp-shared-fs"
  encrypted      = true

  tags = {
    Name    = "webapp-shared-fs"
    Project = "webapp"
  }
}
