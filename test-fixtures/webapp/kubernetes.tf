# ─────────────────────────────────────────────────────────────────────────────
# Kubernetes layer
# EKS cluster + node groups
# network UUID will be auto-resolved by the generator if a subnet exists
# ─────────────────────────────────────────────────────────────────────────────

# EKS control plane  →  upcloud_kubernetes_cluster
resource "aws_eks_cluster" "main" {
  name    = "webapp-k8s"
  version = "1.29"

  role_arn = aws_iam_role.eks.arn

  vpc_config {
    subnet_ids              = [aws_subnet.private_a.id, aws_subnet.private_b.id]
    endpoint_private_access = true
    endpoint_public_access  = true
    public_access_cidrs     = ["0.0.0.0/0"]
  }

  tags = {
    Name    = "webapp-k8s"
    Project = "webapp"
  }
}

# General-purpose node group  →  upcloud_kubernetes_node_group
resource "aws_eks_node_group" "general" {
  cluster_name    = aws_eks_cluster.main.name
  node_group_name = "webapp-general"
  node_role_arn   = aws_iam_role.eks_node.arn

  subnet_ids     = [aws_subnet.private_a.id, aws_subnet.private_b.id]
  instance_types = ["t3.medium"]

  scaling_config {
    desired_size = 3
    min_size     = 2
    max_size     = 8
  }

  tags = {
    Name    = "webapp-general-ng"
    Project = "webapp"
  }
}

# CPU-optimised node group for compute-heavy workloads
resource "aws_eks_node_group" "compute" {
  cluster_name    = aws_eks_cluster.main.name
  node_group_name = "webapp-compute"
  node_role_arn   = aws_iam_role.eks_node.arn

  subnet_ids     = [aws_subnet.private_a.id]
  instance_types = ["c5.xlarge"]

  scaling_config {
    desired_size = 2
    min_size     = 1
    max_size     = 6
  }

  tags = {
    Name    = "webapp-compute-ng"
    Project = "webapp"
  }
}
