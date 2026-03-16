# ─────────────────────────────────────────────────────────────────────────────
# Compute — key pairs, EC2 instances, EBS volumes
# ─────────────────────────────────────────────────────────────────────────────

resource "aws_key_pair" "deploy" {
  key_name   = "saas-deploy"
  public_key = var.ssh_public_key
}

# Web servers — 2 instances, two security groups (web + monitoring)
# The generator merges both SGs into one upcloud_firewall_rules resource.
resource "aws_instance" "web" {
  count         = 2
  ami           = "ami-0c55b159cbfafe1f0"
  instance_type = var.web_instance_type
  subnet_id     = aws_subnet.public_a.id
  key_name      = aws_key_pair.deploy.key_name

  vpc_security_group_ids = [
    aws_security_group.web.id,
    aws_security_group.monitoring.id,
  ]

  root_block_device {
    volume_size = 20
    volume_type = "gp3"
  }

  tags = {
    Name        = "saas-web-${count.index + 1}"
    Role        = "web"
    Environment = var.environment
  }
}

# API servers — 2 instances
resource "aws_instance" "api" {
  count         = 2
  ami           = "ami-0c55b159cbfafe1f0"
  instance_type = var.app_instance_type
  subnet_id     = aws_subnet.private_a.id
  key_name      = aws_key_pair.deploy.key_name

  vpc_security_group_ids = [
    aws_security_group.api.id,
    aws_security_group.monitoring.id,
  ]

  root_block_device {
    volume_size = 40
    volume_type = "gp3"
  }

  tags = {
    Name        = "saas-api-${count.index + 1}"
    Role        = "api"
    Environment = var.environment
  }
}

# Persistent data volume for API servers
resource "aws_ebs_volume" "api_data" {
  availability_zone = "eu-west-1a"
  size              = 200
  type              = "gp3"
  tags              = { Name = "saas-api-data" }
}

resource "aws_volume_attachment" "api_data" {
  device_name = "/dev/xvdf"
  volume_id   = aws_ebs_volume.api_data.id
  instance_id = aws_instance.api[0].id
}
