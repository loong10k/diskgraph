//! 当前执行代次的纯停止资格；来源：原生 Rust ControlStore / PF-06。

use crate::{ControlStore, Result};
use rusqlite::params;

impl ControlStore {
    /// 仅为当前 Running owner/fence 持久记录取消，不授予任何读取或执行权限。
    /// 参数：job_id 为任务，owner/fence 必须来自执行器自己的成功认领。
    /// 返回：同一 Running 代次为 true；不存在、非 Running 或凭证不匹配为 false。
    /// 不复验 OperationView，不续租、不更改终态；远程适配器不得接受客户端提供此资格。
    pub fn request_cancel_generation(
        &mut self,
        job_id: &str,
        owner: &str,
        fence: u64,
    ) -> Result<bool> {
        let Ok(fence) = i64::try_from(fence) else {
            return Ok(false);
        };
        let changed = self.connection.execute(
            "UPDATE jobs SET cancel_requested=1
             WHERE job_id=?1 AND owner=?2 AND fencing_token=?3 AND state='running'",
            params![job_id, owner, fence],
        )?;
        Ok(changed == 1)
    }
}
