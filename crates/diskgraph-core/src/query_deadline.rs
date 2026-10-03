use crate::{BusinessError, QueryBudget};
use std::time::{Duration, Instant};

/// 在首次准备/授权之前建立一个 checked 绝对期限。
/// 参数：budget 沿用 typed 查询的正数契约，不额外限制为 1000ms。
/// 返回：同一次请求共享的 Instant，零额度或不可表示的期限返回参数错误。
pub fn query_deadline(budget: QueryBudget) -> Result<Instant, BusinessError> {
    budget.validated()?;
    Instant::now()
        .checked_add(Duration::from_millis(budget.deadline_ms))
        .ok_or(BusinessError::InvalidArgument)
}
