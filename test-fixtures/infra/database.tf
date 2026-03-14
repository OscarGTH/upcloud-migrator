resource "aws_db_instance" "postgres" {
  identifier        = "myapp-db"
  engine            = "postgres"
  engine_version    = "15.3"
  instance_class    = "db.t3.medium"
  allocated_storage = 20
  db_name           = "myapp"
  username          = "admin"
  password          = "changeme"
}

resource "aws_elasticache_cluster" "redis" {
  cluster_id        = "myapp-cache"
  engine            = "redis"
  node_type         = "cache.t3.micro"
  num_cache_nodes   = 1
}
