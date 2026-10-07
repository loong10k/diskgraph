//! 单图库事务的固定 Git 版本及唯一回执发布；来源：原生 Rust EC-02 / RT-01。
use crate::{Result, SqliteSnapshotStore, StoreError};
use diskgraph_core::{
    CollectorBatch, GitEvidenceJobInput, JobPublicationReceipt, QueryBudget, QueryReadBudget,
};
use rusqlite::{TransactionBehavior, params};
use std::time::{Duration, Instant};

impl SqliteSnapshotStore {
    /// 参数：真实 job、原输入、(fence, Unix毫秒)、新 revision、受控批次与原运行纯检查。
    /// 返回：同事务唯一回执，任一末检/CAS/来源错误全部回滚；旧回执不会被替换或二次发布。
    /// check 必须只读原 expiry/cancel/lease/Instant，不得重入外层控制事务或执行源 I/O。
    pub fn publish_git_collector_revision_checked(
        &mut self,
        job_id: &str,
        input: &GitEvidenceJobInput,
        publishing: (u64, u64),
        revision_id: &str,
        batch: &CollectorBatch,
        mut check: impl FnMut() -> Result<()>,
    ) -> Result<JobPublicationReceipt> {
        check()?;
        crate::git_collector_batch_validation::validate(input, batch)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check()?;
        if let Some(receipt) = crate::job_receipt_query::read(&tx, job_id)? {
            if receipt.input() != input {
                return Err(StoreError::InvalidGraph(
                    "job receipt target mismatch".into(),
                ));
            }
            return Ok(receipt);
        }
        let mut statement=tx.prepare("SELECT r.snapshot_id,s.root_key,o.server_id,o.scope_id
            FROM graph_revisions r JOIN snapshots s ON s.id=r.snapshot_id JOIN revision_authorized_ownership o ON o.revision_id=r.revision_id
            WHERE r.revision_id=?1")?;
        let mut rows = statement.query([input.base_revision_id()])?;
        let row = rows
            .next()?
            .ok_or_else(|| StoreError::RevisionNotFound(input.base_revision_id().into()))?;
        let bytes = (0..4).try_fold(0_usize, |total, column| -> Result<usize> {
            total
                .checked_add(
                    row.get_ref(column)?
                        .as_bytes()
                        .map_err(|_| StoreError::InvalidGraph("invalid collector base".into()))?
                        .len(),
                )
                .ok_or(StoreError::IntegerOverflow)
        })?;
        if bytes > 16384 {
            return Err(StoreError::BudgetExceeded);
        }
        let snapshot: String = row.get(0)?;
        let root: String = row.get(1)?;
        if row.get::<_, String>(2)? != input.server_id().as_str()
            || row.get::<_, String>(3)? != input.scope_id().as_str()
        {
            return Err(StoreError::Conflict("Git base ownership mismatch".into()));
        }
        drop(rows);
        drop(statement);
        check()?;
        crate::git_collector_base::require_current(&tx, input, &root)?;
        check()?;
        let directory: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM nodes WHERE snapshot_id=?1 AND id=?2
            AND COALESCE(kind,json_extract(NULLIF(node_json,''),'$.kind'))='directory'
            AND COALESCE(read_error,json_extract(NULLIF(node_json,''),'$.read_error'),0)=0)",
            params![snapshot, input.node_id() as i64],
            |r| r.get(0),
        )?;
        if !directory || batch.run.snapshot_id != snapshot {
            return Err(StoreError::InvalidGraph(
                "Git target is not the bound readable directory".into(),
            ));
        }
        let existing_run: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM collector_runs WHERE run_id=?1)",
            [&batch.run.run_id],
            |r| r.get(0),
        )?;
        if existing_run {
            return Err(StoreError::InvalidGraph(
                "Git run already exists without a receipt".into(),
            ));
        }
        for entity in &batch.entities {
            check()?;
            let exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM entities WHERE snapshot_id=?1 AND entity_id=?2)",
                params![snapshot, entity.entity_id],
                |r| r.get(0),
            )?;
            if exists {
                return Err(StoreError::InvalidGraph(
                    "Git batch must own new immutable entities".into(),
                ));
            }
        }
        // 原任务时间由 check 复验；局部一秒仅限制发布读取准备，不能续期原运行预算。
        let budget = QueryBudget {
            max_nodes: 1,
            max_edges: 4095,
            max_response_bytes: (1 << 20) - 16384,
            deadline_ms: 1000,
            ..QueryBudget::default()
        };
        let mut reads = QueryReadBudget::new(budget, Instant::now() + Duration::from_secs(1))
            .map_err(|_| StoreError::BudgetExceeded)?;
        let mut selected = crate::git_collector_selection::read(
            &tx,
            input.base_revision_id(),
            input.node_id(),
            &mut reads,
            &mut check,
        )?;
        selected.push((batch.run.run_id.clone(), "active".into()));
        crate::collector_batch_writer::write_batch(
            &tx,
            &snapshot,
            &batch.run,
            &batch.entities,
            &batch.evidence,
            &batch.edges,
        )?;
        check()?;
        tx.execute("INSERT INTO graph_revisions(revision_id,snapshot_id,published_at_unix_ms,writer_generation,locator_writer_generation,native_observation_writer_generation) VALUES(?1,?2,?3,10,11,12)",
            params![revision_id,snapshot,publishing.1 as i64])?;
        tx.execute(
            "INSERT INTO revision_ownership VALUES(?1,?2,?3)",
            params![
                revision_id,
                input.server_id().as_str(),
                input.scope_id().as_str()
            ],
        )?;
        // 原 run_json 已借用准入并验证，无需 bind_runs 再无界拥有整份 JSON。
        for (run, role) in selected {
            check()?;
            tx.execute(
                "INSERT INTO revision_runs VALUES(?1,?2,?3)",
                params![revision_id, run, role],
            )?;
        }
        crate::collector_protocol::seal(&tx, revision_id)?;
        crate::git_collector_base::require_latest(&tx, input, revision_id)?;
        // 兼容显示根只在仍指向本基线时推进，其他 namespace 的根指针必须保持不变。
        tx.execute(
            "UPDATE latest_revision SET revision_id=?2 WHERE root_key=?1 AND revision_id=?3",
            params![root, revision_id, input.base_revision_id()],
        )?;
        let receipt = JobPublicationReceipt::new(
            job_id.into(),
            input.clone(),
            snapshot,
            revision_id.into(),
            batch.run.run_id.clone(),
            publishing,
        )
        .map_err(|message| StoreError::InvalidGraph(message.into()))?;
        let encoded = serde_json::to_string(&receipt)?;
        if encoded.len() > 16384 {
            return Err(StoreError::BudgetExceeded);
        }
        tx.execute(
            "INSERT INTO job_publication_receipts VALUES(?1,1,?2,?3,?4,?5,?6,?7,?8,?9,13)",
            params![
                job_id,
                receipt.input_sha256(),
                input.server_id().as_str(),
                input.scope_id().as_str(),
                receipt.snapshot_id(),
                revision_id,
                receipt.run_id(),
                input.node_id() as i64,
                encoded
            ],
        )?;
        // 实际最后一条 SQL 之后、真正 tx.commit 之前再检查，拒绝会连回执一起回滚。
        check()?;
        tx.commit()?;
        Ok(receipt)
    }
}
