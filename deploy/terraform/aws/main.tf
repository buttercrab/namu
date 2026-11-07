# NAMU on AWS: an EKS cluster with on-demand nodes for the orchestrator and spot nodes for
# workers, MemoryDB for Redis, an S3 bucket for task packages, and the orchestrator's access to
# it. The Helm chart (deploy/helm/namu) installs NAMU on the cluster; see
# docs/operations/kubernetes.md.

data "aws_availability_zones" "available" {
  state = "available"
}

locals {
  azs = slice(data.aws_availability_zones.available.names, 0, 3)
}

module "vpc" {
  source  = "terraform-aws-modules/vpc/aws"
  version = "~> 6.0"

  name            = var.name
  cidr            = var.vpc_cidr
  azs             = local.azs
  private_subnets = [for i, _ in local.azs : cidrsubnet(var.vpc_cidr, 4, i)]
  public_subnets  = [for i, _ in local.azs : cidrsubnet(var.vpc_cidr, 8, 48 + i)]

  enable_nat_gateway = true
  single_nat_gateway = true

  public_subnet_tags  = { "kubernetes.io/role/elb" = 1 }
  private_subnet_tags = { "kubernetes.io/role/internal-elb" = 1 }
}

module "eks" {
  source  = "terraform-aws-modules/eks/aws"
  version = "~> 21.0"

  name                   = var.name
  kubernetes_version     = var.kubernetes_version
  vpc_id                 = module.vpc.vpc_id
  subnet_ids             = module.vpc.private_subnets
  endpoint_public_access = true
  enable_irsa            = true

  enable_cluster_creator_admin_permissions = true

  addons = {
    coredns                = {}
    kube-proxy             = {}
    vpc-cni                = { before_compute = true }
    eks-pod-identity-agent = { before_compute = true }
  }

  eks_managed_node_groups = {
    # The orchestrator, and the cluster's own pods: never reclaimed.
    system = {
      ami_type       = "AL2023_ARM_64_STANDARD"
      instance_types = var.orchestrator_instance_types
      capacity_type  = "ON_DEMAND"
      min_size       = 1
      max_size       = 3
      desired_size   = 2
      labels         = { "namu.dev/pool" = "system" }
    }
    # Workers, on spot: a reclaimed node's workers drain (see the chart's
    # worker.terminationGracePeriodSeconds), and a killed one is taken over from the log.
    workers = {
      ami_type       = "AL2023_ARM_64_STANDARD"
      instance_types = var.worker_instance_types
      capacity_type  = "SPOT"
      min_size       = var.worker_nodes.min
      max_size       = var.worker_nodes.max
      desired_size   = var.worker_nodes.desired
      labels         = { "namu.dev/pool" = "workers" }
      taints = {
        workers = { key = "namu.dev/pool", value = "workers", effect = "NO_SCHEDULE" }
      }
    }
  }
}

# --- Redis: MemoryDB, durable across a multi-AZ transaction log -----------------------------

resource "random_password" "redis" {
  length  = 32
  special = false
}

resource "aws_memorydb_user" "namu" {
  user_name     = var.name
  access_string = "on ~* &* +@all"
  authentication_mode {
    type      = "password"
    passwords = [random_password.redis.result]
  }
}

resource "aws_memorydb_acl" "namu" {
  name       = var.name
  user_names = [aws_memorydb_user.namu.user_name]
}

resource "aws_memorydb_subnet_group" "namu" {
  name       = var.name
  subnet_ids = module.vpc.private_subnets
}

resource "aws_security_group" "redis" {
  name   = "${var.name}-redis"
  vpc_id = module.vpc.vpc_id
  ingress {
    description     = "Redis from the cluster's nodes"
    from_port       = 6379
    to_port         = 6379
    protocol        = "tcp"
    security_groups = [module.eks.node_security_group_id]
  }
}

resource "aws_memorydb_cluster" "namu" {
  name                     = var.name
  node_type                = var.memorydb_node_type
  num_shards               = 1
  num_replicas_per_shard   = var.memorydb_replicas
  acl_name                 = aws_memorydb_acl.namu.name
  subnet_group_name        = aws_memorydb_subnet_group.namu.name
  security_group_ids       = [aws_security_group.redis.id]
  tls_enabled              = true
  snapshot_retention_limit = 7
}

# The URL NAMU reaches it by, password included: put it in the `namu-redis` Secret.
resource "aws_secretsmanager_secret" "redis_url" {
  name = "${var.name}/redis-url"
}

resource "aws_secretsmanager_secret_version" "redis_url" {
  secret_id = aws_secretsmanager_secret.redis_url.id
  secret_string = format(
    "rediss://%s:%s@%s:%d/",
    aws_memorydb_user.namu.user_name,
    random_password.redis.result,
    aws_memorydb_cluster.namu.cluster_endpoint[0].address,
    aws_memorydb_cluster.namu.cluster_endpoint[0].port,
  )
}

# --- Task packages: S3 ------------------------------------------------------------------------

resource "aws_s3_bucket" "artifacts" {
  bucket_prefix = "${var.name}-artifacts-"
}

resource "aws_s3_bucket_public_access_block" "artifacts" {
  bucket                  = aws_s3_bucket.artifacts.id
  block_public_acls       = true
  block_public_policy     = true
  ignore_public_acls      = true
  restrict_public_buckets = true
}

resource "aws_s3_bucket_versioning" "artifacts" {
  bucket = aws_s3_bucket.artifacts.id
  versioning_configuration {
    status = "Enabled"
  }
}

resource "aws_s3_bucket_server_side_encryption_configuration" "artifacts" {
  bucket = aws_s3_bucket.artifacts.id
  rule {
    apply_server_side_encryption_by_default {
      sse_algorithm = "aws:kms"
    }
  }
}

# The orchestrator's service account may read and write task packages, and nothing else.
module "orchestrator_role" {
  source  = "terraform-aws-modules/iam/aws//modules/iam-role-for-service-accounts"
  version = "~> 6.0"

  name = "${var.name}-orchestrator"
  oidc_providers = {
    eks = {
      provider_arn               = module.eks.oidc_provider_arn
      namespace_service_accounts = ["${var.namespace}:${var.service_account}"]
    }
  }
  policies = { artifacts = aws_iam_policy.artifacts.arn }
}

resource "aws_iam_policy" "artifacts" {
  name = "${var.name}-artifacts"
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Effect   = "Allow"
        Action   = ["s3:GetObject", "s3:PutObject"]
        Resource = "${aws_s3_bucket.artifacts.arn}/*"
      },
      {
        Effect   = "Allow"
        Action   = ["s3:ListBucket"]
        Resource = aws_s3_bucket.artifacts.arn
      },
    ]
  })
}
