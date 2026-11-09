variable "name" {
  description = "The cluster's name: the EKS cluster, MemoryDB, and the bucket's prefix."
  type        = string
  default     = "namu"
}

variable "region" {
  type    = string
  default = "us-east-1"
}

variable "vpc_cidr" {
  type    = string
  default = "10.42.0.0/16"
}

variable "kubernetes_version" {
  type    = string
  default = "1.35"
}

variable "orchestrator_instance_types" {
  description = "On-demand nodes for the orchestrator and the cluster's own pods."
  type        = list(string)
  default     = ["m7g.large"]
}

variable "worker_instance_types" {
  description = "Spot nodes for workers: several types, so spot capacity is found. Graviton (arm64) by default; the images build for both."
  type        = list(string)
  default     = ["c7g.2xlarge", "c6g.2xlarge", "m7g.2xlarge", "m6g.2xlarge"]
}

variable "worker_nodes" {
  description = "Worker node group size: min, desired, max."
  type        = object({ min = number, desired = number, max = number })
  default     = { min = 1, desired = 3, max = 20 }
}

variable "memorydb_node_type" {
  type    = string
  default = "db.r7g.large"
}

variable "memorydb_replicas" {
  description = "Replicas per shard: one or more, so a node's failure loses no committed write."
  type        = number
  default     = 1
}

variable "namespace" {
  description = "The Kubernetes namespace NAMU is installed in (for the orchestrator's IAM role)."
  type        = string
  default     = "namu"
}

variable "force_destroy" {
  description = "Let `terraform destroy` take everything: the bucket with its task packages in it, and the Redis URL's secret at once, rather than after its 30-day recovery window (so the name is free again). For clusters made to be thrown away, such as a test's."
  type        = bool
  default     = false
}

variable "service_account" {
  description = "The orchestrator's service account: `<release>-orchestrator` for a Helm release named namu or namu-*, else `<release>-namu-orchestrator`."
  type        = string
  default     = "namu-orchestrator"
}
