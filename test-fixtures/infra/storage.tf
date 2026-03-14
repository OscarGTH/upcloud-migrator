resource "aws_s3_bucket" "media" {
  bucket = "my-app-media-bucket"
  tags = {
    Name = "media-bucket"
  }
}

resource "aws_s3_bucket_policy" "media_policy" {
  bucket = aws_s3_bucket.media.id
  policy = "{}"
}

resource "aws_ebs_volume" "data" {
  availability_zone = "us-east-1a"
  size              = 100
  type              = "gp3"
}

resource "aws_efs_file_system" "shared" {
  creation_token = "shared-fs"
}
