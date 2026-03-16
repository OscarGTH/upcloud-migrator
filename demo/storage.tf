# ─────────────────────────────────────────────────────────────────────────────
# Storage — S3 buckets, EFS file system
# ─────────────────────────────────────────────────────────────────────────────

# Static assets + user uploads
resource "aws_s3_bucket" "assets" {
  bucket = "saas-${var.environment}-assets"
  tags   = { Name = "saas-assets", Environment = var.environment }
}

# Terraform state (separate bucket)
resource "aws_s3_bucket" "tfstate" {
  bucket = "saas-${var.environment}-tfstate"
  tags   = { Name = "saas-tfstate" }
}

# Shared file system mounted on all API servers
resource "aws_efs_file_system" "shared" {
  creation_token = "saas-shared-fs"
  encrypted      = true

  tags = { Name = "saas-shared-fs", Environment = var.environment }
}
