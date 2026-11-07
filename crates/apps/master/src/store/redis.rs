//! The control plane's state in Redis.
//!
//! Keys:
//! - `namu:task:{id}:{version}`, `namu:workflow:{id}:{version}`: JSON, set once (`SET NX`).
//! - `namu:member:{id}`: a member's record, expiring with its lease; `namu:alive`, the set of
//!   workers not yet buried; `namu:successors`, dead worker -> successor; `namu:worker_names`.
//! - `namu:run:{id}:spec` (JSON, set once), `namu:run:{id}:owner`, `namu:run:{id}:end` (JSON, set
//!   once), and `namu:running`, the set of runs that have not ended.
//! - `namu:run:{id}:results` and `:failures`, lists in the order reports arrived, and `:reported`,
//!   the set of branches whose report is in one of them.
//!
//! Ids never contain `:` (the control plane checks), so keys cannot collide. Nothing scans the
//! keyspace: members are read through `namu:alive`, runs through `namu:running`.

use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::Context;
use async_trait::async_trait;
use namu_proto::{Member, TaskManifest, WorkflowUploadRequest};
use redis::AsyncCommands;
use redis::aio::ConnectionManager;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value as Json;
use uuid::Uuid;

use super::{Inserted, Reported, RunEnd, RunRecord, RunSpec, Store};

const WORKER_SEQ: &str = "namu:worker_seq";
const WORKER_NAMES: &str = "namu:worker_names";
const OWNER_CURSOR: &str = "namu:owner_cursor";
const ALIVE: &str = "namu:alive";
const SUCCESSORS: &str = "namu:successors";
const RUNNING: &str = "namu:running";

fn task_key(id: &str, version: &str) -> String {
    format!("namu:task:{id}:{version}")
}

fn workflow_key(id: &str, version: &str) -> String {
    format!("namu:workflow:{id}:{version}")
}

fn member_key(id: u64) -> String {
    format!("namu:member:{id}")
}

fn run_key(id: Uuid, part: &str) -> String {
    format!("namu:run:{id}:{part}")
}

#[derive(Clone)]
pub struct RedisStore {
    redis: ConnectionManager,
}

impl RedisStore {
    pub async fn connect(url: &str) -> anyhow::Result<Self> {
        let client = redis::Client::open(url).with_context(|| format!("Redis URL {url:?}"))?;
        let redis = ConnectionManager::new(client)
            .await
            .with_context(|| format!("connecting to Redis at {url}"))?;
        Ok(Self { redis })
    }

    fn conn(&self) -> ConnectionManager {
        self.redis.clone()
    }

    /// Sets `key` to `value` unless it is set; returns what it held if it was.
    async fn insert<T: Serialize + DeserializeOwned>(
        &self,
        key: &str,
        value: &T,
    ) -> anyhow::Result<Inserted<T>> {
        let set: Option<String> = redis::cmd("SET")
            .arg(key)
            .arg(serde_json::to_string(value)?)
            .arg("NX")
            .query_async(&mut self.conn())
            .await?;
        if set.is_some() {
            return Ok(Inserted::New);
        }
        let existing = self
            .get(key)
            .await?
            .with_context(|| format!("{key} vanished"))?;
        Ok(Inserted::Existing(existing))
    }

    async fn get<T: DeserializeOwned>(&self, key: &str) -> anyhow::Result<Option<T>> {
        let raw: Option<String> = self.conn().get(key).await?;
        raw.map(|raw| serde_json::from_str(&raw).with_context(|| format!("decoding {key}")))
            .transpose()
    }
}

/// Adds a report to its list unless its branch's is in already: `KEYS` are the set of reported
/// branches and the list, `ARGV` the branch and the report.
const RECORD: &str = r#"
if redis.call('SADD', KEYS[1], ARGV[1]) == 1 then
  redis.call('RPUSH', KEYS[2], ARGV[2])
  return 1
end
return 0
"#;

/// The `limit` items of `list` from the `after`th on.
async fn page(
    redis: &mut ConnectionManager,
    list: &str,
    after: u64,
    limit: u64,
) -> anyhow::Result<Vec<String>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let (start, stop) = (after as isize, (after + limit - 1) as isize);
    Ok(redis.lrange(list, start, stop).await?)
}

fn millis(ttl: Duration) -> u64 {
    ttl.as_millis().try_into().unwrap_or(u64::MAX)
}

#[async_trait]
impl Store for RedisStore {
    async fn insert_task(&self, manifest: &TaskManifest) -> anyhow::Result<Inserted<TaskManifest>> {
        self.insert(&task_key(&manifest.task_id, &manifest.version), manifest)
            .await
    }

    async fn task(&self, id: &str, version: &str) -> anyhow::Result<Option<TaskManifest>> {
        self.get(&task_key(id, version)).await
    }

    async fn insert_workflow(
        &self,
        workflow: &WorkflowUploadRequest,
    ) -> anyhow::Result<Inserted<WorkflowUploadRequest>> {
        self.insert(&workflow_key(&workflow.id, &workflow.version), workflow)
            .await
    }

    async fn workflow(
        &self,
        id: &str,
        version: &str,
    ) -> anyhow::Result<Option<WorkflowUploadRequest>> {
        self.get(&workflow_key(id, version)).await
    }

    async fn next_worker_id(&self) -> anyhow::Result<u64> {
        Ok(self.conn().incr(WORKER_SEQ, 1).await?)
    }

    async fn join(&self, member: &Member, ttl: Duration) -> anyhow::Result<()> {
        let record = serde_json::to_string(member)?;
        redis::pipe()
            .atomic()
            .pset_ex(member_key(member.id), record, millis(ttl))
            .ignore()
            .hset(WORKER_NAMES, member.id, &member.name)
            .ignore()
            .sadd(ALIVE, member.id)
            .ignore()
            .query_async::<()>(&mut self.conn())
            .await?;
        Ok(())
    }

    async fn renew(&self, id: u64, ttl: Duration) -> anyhow::Result<bool> {
        Ok(self
            .conn()
            .pexpire(member_key(id), millis(ttl) as i64)
            .await?)
    }

    async fn update_member(&self, member: &Member, ttl: Duration) -> anyhow::Result<bool> {
        let set: Option<String> = redis::cmd("SET")
            .arg(member_key(member.id))
            .arg(serde_json::to_string(member)?)
            .arg("XX")
            .arg("PX")
            .arg(millis(ttl))
            .query_async(&mut self.conn())
            .await?;
        Ok(set.is_some())
    }

    async fn end_lease(&self, id: u64) -> anyhow::Result<()> {
        let _: () = self.conn().del(member_key(id)).await?;
        Ok(())
    }

    async fn members(&self) -> anyhow::Result<Vec<Member>> {
        let alive = self.alive().await?;
        if alive.is_empty() {
            return Ok(Vec::new());
        }
        let keys: Vec<String> = alive.iter().map(|&id| member_key(id)).collect();
        let records: Vec<Option<String>> = self.conn().mget(&keys).await?;
        let mut members = records
            .into_iter()
            .flatten()
            .map(|record| serde_json::from_str(&record).context("decoding a member"))
            .collect::<anyhow::Result<Vec<Member>>>()?;
        members.sort_by_key(|member| member.id);
        Ok(members)
    }

    async fn alive(&self) -> anyhow::Result<Vec<u64>> {
        Ok(self.conn().smembers(ALIVE).await?)
    }

    async fn bury(&self, dead: u64, successor: u64) -> anyhow::Result<()> {
        redis::pipe()
            .atomic()
            .srem(ALIVE, dead)
            .ignore()
            .hset(SUCCESSORS, dead, successor)
            .ignore()
            .query_async::<()>(&mut self.conn())
            .await?;
        Ok(())
    }

    async fn successors(&self) -> anyhow::Result<BTreeMap<u64, u64>> {
        Ok(self.conn().hgetall(SUCCESSORS).await?)
    }

    async fn worker_names(&self, ids: &[u64]) -> anyhow::Result<Vec<Option<String>>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let names: Vec<Option<String>> = redis::cmd("HMGET")
            .arg(WORKER_NAMES)
            .arg(ids)
            .query_async(&mut self.conn())
            .await?;
        Ok(names)
    }

    async fn next_owner(&self) -> anyhow::Result<u64> {
        Ok(self.conn().incr(OWNER_CURSOR, 1).await?)
    }

    async fn create_run(&self, id: Uuid, spec: &RunSpec, owner: u64) -> anyhow::Result<()> {
        redis::pipe()
            .atomic()
            .set(run_key(id, "spec"), serde_json::to_string(spec)?)
            .ignore()
            .set(run_key(id, "owner"), owner)
            .ignore()
            .sadd(RUNNING, id.to_string())
            .ignore()
            .query_async::<()>(&mut self.conn())
            .await?;
        Ok(())
    }

    async fn run(&self, id: Uuid) -> anyhow::Result<Option<RunRecord>> {
        let keys = [
            run_key(id, "spec"),
            run_key(id, "owner"),
            run_key(id, "end"),
        ];
        let ((spec, owner, end), results, failures): (
            (Option<String>, Option<u64>, Option<String>),
            u64,
            u64,
        ) = redis::pipe()
            .cmd("MGET")
            .arg(&keys)
            .llen(run_key(id, "results"))
            .llen(run_key(id, "failures"))
            .query_async(&mut self.conn())
            .await?;
        let (Some(spec), Some(owner)) = (spec, owner) else {
            return Ok(None);
        };
        Ok(Some(RunRecord {
            spec: serde_json::from_str(&spec).context("decoding a run")?,
            owner,
            results,
            failures,
            end: end
                .map(|end| serde_json::from_str(&end).context("decoding a run's end"))
                .transpose()?,
        }))
    }

    async fn running(&self) -> anyhow::Result<Vec<(Uuid, u64)>> {
        let ids: Vec<String> = self.conn().smembers(RUNNING).await?;
        let ids = ids
            .iter()
            .map(|id| id.parse::<Uuid>().with_context(|| format!("run id {id:?}")))
            .collect::<anyhow::Result<Vec<_>>>()?;
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let keys: Vec<String> = ids.iter().map(|&id| run_key(id, "owner")).collect();
        let owners: Vec<Option<u64>> = redis::cmd("MGET")
            .arg(&keys)
            .query_async(&mut self.conn())
            .await?;
        Ok(ids
            .into_iter()
            .zip(owners)
            .filter_map(|(id, owner)| Some((id, owner?)))
            .collect())
    }

    async fn set_owner(&self, id: Uuid, owner: u64) -> anyhow::Result<()> {
        let _: () = self.conn().set(run_key(id, "owner"), owner).await?;
        Ok(())
    }

    async fn end_run(&self, id: Uuid, end: &RunEnd) -> anyhow::Result<()> {
        redis::pipe()
            .atomic()
            .set(run_key(id, "end"), serde_json::to_string(end)?)
            .ignore()
            .srem(RUNNING, id.to_string())
            .ignore()
            .query_async::<()>(&mut self.conn())
            .await?;
        Ok(())
    }

    async fn record(&self, id: Uuid, branch: &str, report: &Reported) -> anyhow::Result<bool> {
        let (list, item) = match report {
            Reported::Result(json) => ("results", serde_json::to_string(json)?),
            Reported::Failure(failure) => ("failures", failure.clone()),
        };
        let added: u8 = redis::Script::new(RECORD)
            .key(run_key(id, "reported"))
            .key(run_key(id, list))
            .arg(branch)
            .arg(item)
            .invoke_async(&mut self.conn())
            .await?;
        Ok(added == 1)
    }

    async fn results(&self, id: Uuid, after: u64, limit: u64) -> anyhow::Result<Vec<Json>> {
        page(&mut self.conn(), &run_key(id, "results"), after, limit)
            .await?
            .iter()
            .map(|json| serde_json::from_str(json).context("decoding a result"))
            .collect()
    }

    async fn failures(&self, id: Uuid, after: u64, limit: u64) -> anyhow::Result<Vec<String>> {
        page(&mut self.conn(), &run_key(id, "failures"), after, limit).await
    }

    async fn replace_reports(
        &self,
        id: Uuid,
        results: &[Json],
        failures: &[String],
    ) -> anyhow::Result<()> {
        let (results_key, failures_key) = (run_key(id, "results"), run_key(id, "failures"));
        let results = results
            .iter()
            .map(serde_json::to_string)
            .collect::<Result<Vec<_>, _>>()?;
        let mut pipe = redis::pipe();
        pipe.atomic()
            .del(&results_key)
            .ignore()
            .del(&failures_key)
            .ignore();
        if !results.is_empty() {
            pipe.rpush(&results_key, results).ignore();
        }
        if !failures.is_empty() {
            pipe.rpush(&failures_key, failures).ignore();
        }
        pipe.query_async::<()>(&mut self.conn()).await?;
        Ok(())
    }
}
