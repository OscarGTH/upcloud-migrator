# ─────────────────────────────────────────────────────────────────────────────
# Compute layer
# Key pairs, EC2 instances (with count), autoscaling
# ─────────────────────────────────────────────────────────────────────────────

# SSH key pair  →  login block embedded in upcloud_server
# When public_key is set the generator auto-resolves the login block.
resource "aws_key_pair" "deployer" {
  key_name   = "webapp-deployer"
  public_key = "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABgQC3vE9i7uBuB5rX1sZ deployer@example.com"
}

# Web servers — 2 instances  →  upcloud_server with count = 2
resource "aws_instance" "web" {
  count         = 2
  ami           = "ami-0c55b159cbfafe1f0"
  instance_type = "t3.small"
  subnet_id     = aws_subnet.public_a.id
  key_name      = aws_key_pair.deployer.key_name

  vpc_security_group_ids = [aws_security_group.web.id]

  root_block_device {
    volume_size = 20
    volume_type = "gp3"
  }

  tags = {
    Name    = "webapp-web-${count.index + 1}"
    Role    = "web"
    Project = "webapp"
  }
}

# App servers — 2 instances  →  upcloud_server with count = 2
resource "aws_instance" "app" {
  count         = 2
  ami           = "ami-0c55b159cbfafe1f0"
  instance_type = "t3.medium"
  subnet_id     = aws_subnet.private_a.id
  key_name      = aws_key_pair.deployer.key_name

  vpc_security_group_ids = [aws_security_group.app.id]

  root_block_device {
    volume_size = 30
    volume_type = "gp3"
  }

  tags = {
    Name    = "webapp-app-${count.index + 1}"
    Role    = "app"
    Project = "webapp"
  }
}

# Database server — single instance  →  upcloud_server
resource "aws_instance" "database" {
  ami           = "ami-0c55b159cbfafe1f0"
  instance_type = "m5.large"
  subnet_id     = aws_subnet.db_a.id
  key_name      = aws_key_pair.deployer.key_name

  vpc_security_group_ids = [aws_security_group.db.id]

  root_block_device {
    volume_size = 50
    volume_type = "io1"
    iops        = 1000
  }

  tags = {
    Name    = "webapp-database"
    Role    = "database"
    Project = "webapp"
  }
}

# Bastion host — no key pair attribute to show the generic SSH TODO placeholder
resource "aws_instance" "bastion" {
  ami           = "ami-0c55b159cbfafe1f0"
  instance_type = "t3.micro"
  subnet_id     = aws_subnet.public_a.id

  tags = {
    Name    = "webapp-bastion"
    Role    = "bastion"
    Project = "webapp"
  }
}

# Additional EBS volumes  →  upcloud_storage
resource "aws_ebs_volume" "app_data" {
  availability_zone = "eu-west-1a"
  size              = 100
  type              = "gp3"

  tags = { Name = "webapp-app-data" }
}

resource "aws_ebs_volume" "db_data" {
  availability_zone = "eu-west-1a"
  size              = 500
  type              = "io1"

  tags = { Name = "webapp-db-data" }
}

resource "aws_ebs_volume" "archive" {
  availability_zone = "eu-west-1a"
  size              = 2000
  type              = "st1"

  tags = { Name = "webapp-archive" }
}

# Autoscaling group  →  Unsupported (shows up in MIGRATION_NOTES.md)
resource "aws_autoscaling_group" "web_asg" {
  name             = "webapp-web-asg"
  min_size         = 2
  max_size         = 8
  desired_capacity = 2

  vpc_zone_identifier = [aws_subnet.public_a.id, aws_subnet.public_b.id]

  tag {
    key                 = "Name"
    value               = "webapp-web-asg"
    propagate_at_launch = true
  }
}
