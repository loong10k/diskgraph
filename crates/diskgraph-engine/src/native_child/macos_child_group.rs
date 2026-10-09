use super::{ChildError, macos_group_view::MacosGroupView};
use std::mem::{size_of, size_of_val};

/// 读取 fresh 私有 session 的完整组资格，保留 Active/AllExited/Unknown 的区别。
/// 参数：leader 为 WNOWAIT 尚保留的本次 child；返回：真实组状态，截断/身份丢失明确 Unknown。
pub(super) fn normal_view(leader: u32) -> MacosGroupView {
    let (view, disappeared_member) = normal_view_once(leader);
    if disappeared_member {
        // 只在原系统调用明确给出非leader ESRCH时放弃旧样本，再完整采集一次。
        // 新样本仍独立执行所有身份/退出/完整集合核验，不把缺项或第二次未知当成功。
        return normal_view_once(leader).0;
    }
    view
}

fn normal_view_once(leader: u32) -> (MacosGroupView, bool) {
    #[cfg(test)]
    super::macos_group_query_tests::before_group_sample();
    let mut pids = [0i32; 1024];
    let Some(count) = list_group(leader, &mut pids) else {
        return unknown("normal group view is unavailable or truncated");
    };
    pids[..count].sort_unstable();
    if pids[..count].windows(2).any(|pair| pair[0] == pair[1]) {
        return unknown("normal group view contains duplicate members");
    }
    let mut saw_leader = false;
    let mut active = false;
    for &pid in &pids[..count] {
        if pid <= 0 {
            return unknown("normal group view contains an invalid pid");
        }
        #[cfg(test)]
        super::macos_group_query_tests::before_member_query(pid);
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        // 安全性：固定 ABI 输出；arg=1 允许查询本 owner 保留的 zombie leader。
        let queried = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                1,
                (&mut info as *mut libc::proc_bsdinfo).cast(),
                size_of::<libc::proc_bsdinfo>() as i32,
            )
        };
        if queried as usize != size_of::<libc::proc_bsdinfo>() {
            let disappeared_member = queried == 0
                && pid as u32 != leader
                && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH);
            let (view, _) = unknown("normal group member query is unavailable or incomplete");
            return (view, disappeared_member);
        }
        // fresh session 的现存组禁止外部 session 加入；保留 leader 防止组号复用。
        // XNU pgrp_add_member 从同一 pgrp 赋 PGID/sessionID，因此完整 PID/PGID
        // 即可绑定本组。getsid 对退出过渡也可能 ESRCH，不能据它把活动误判为未知或退出。
        if info.pbi_pid != pid as u32 || info.pbi_pgid != leader {
            return unknown("normal group member identity changed");
        }
        if pid as u32 == leader {
            if info.pbi_ppid != std::process::id() || info.pbi_status != libc::SZOMB {
                return unknown("normal group retained leader identity was lost");
            }
            saw_leader = true;
        }
        active |= info.pbi_status != libc::SZOMB;
    }
    if !saw_leader {
        return unknown("normal group view omitted the retained leader");
    }
    if active {
        return (MacosGroupView::Active, false);
    }
    // 已确认终止的成员不能再派生进程；保留 leader 防止组号复用。
    // 第二次完整组枚举若变化或容量不足即 Unknown，绝不洗为空组成功。
    let mut after = [0i32; 1024];
    let Some(after_count) = list_group(leader, &mut after) else {
        return unknown("normal group final view is unavailable or truncated");
    };
    after[..after_count].sort_unstable();
    if pids[..count] != after[..after_count] {
        return unknown("normal group view changed during qualification");
    }
    (MacosGroupView::AllExited, false)
}

fn unknown(reason: &'static str) -> (MacosGroupView, bool) {
    (
        MacosGroupView::Unknown(ChildError::Unsupported(reason)),
        false,
    )
}

/// 核验 Darwin 的 EPERM 是否仅因自有组全部为 zombie，不忽略权限失败。
/// 参数：leader 为尚未回收的自有子进程 PID，必须仍出现在所属组中。
/// 返回：两次完整枚举一致且每个成员确认退出时 true；未知、变化或活动成员均 false。
/// XNU killpg1 排除 SZOMB，组存在但无可发送成员时返回 EPERM。
pub(super) fn zombies_only(leader: u32) -> bool {
    terminal_members_only(leader, false)
}

/// 核验原组全部成员已进入退出或 zombie 状态，仅许可保留 owner 后继续轮询。
/// 参数：leader 是尚未 wait 的原 leader；返回：完整双重枚举一致且身份匹配为 true。
/// INEXIT 不代表退出完成，不能据此消费 wait、释放槽或发布扫描结果。
pub(super) fn exiting_only(leader: u32) -> bool {
    terminal_members_only(leader, true)
}

fn terminal_members_only(leader: u32, allow_exiting: bool) -> bool {
    // Darwin 公共 sys/proc_info.h 的 PROC_FLAG_INEXIT=4；当前 libc 未导出此 ABI 常量。
    const PROC_FLAG_INEXIT: u32 = 4;
    // 固定 4 KiB 枚举，不按宿主总进程数分配；等于容量视为截断并拒绝。
    let mut pids = [0i32; 1024];
    let Some(count) = list_group(leader, &mut pids) else {
        return false;
    };
    let mut saw_leader = false;
    for &pid in &pids[..count] {
        if pid <= 0 {
            return false;
        }
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        // XNU 的 arg=1 才会查询 zombie；arg=0 会把保留 leader 误报 ESRCH。
        let queried = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                1,
                (&mut info as *mut libc::proc_bsdinfo).cast(),
                size_of::<libc::proc_bsdinfo>() as i32,
            )
        };
        if queried as usize != size_of::<libc::proc_bsdinfo>()
            || info.pbi_pid != pid as u32
            || info.pbi_pgid != leader
            || !(info.pbi_status == libc::SZOMB
                || (allow_exiting && info.pbi_flags & PROC_FLAG_INEXIT != 0))
        {
            return false;
        }
        if pid as u32 == leader {
            if info.pbi_ppid != std::process::id() {
                return false;
            }
            saw_leader = true;
        }
    }
    if !saw_leader {
        return false;
    }
    // 对退出过渡仅报告 Pending；再次完整枚举防止检查过程中新成员
    // 被第一份快照遗漏；保留 leader、父身份与宿主不外部 wait 的契约仍必需。
    let mut after = [0i32; 1024];
    let Some(after_count) = list_group(leader, &mut after) else {
        return false;
    };
    pids[..count].sort_unstable();
    after[..after_count].sort_unstable();
    pids[..count] == after[..after_count]
}

fn list_group(leader: u32, pids: &mut [i32; 1024]) -> Option<usize> {
    let bytes = unsafe {
        libc::proc_listpids(
            2,
            leader,
            pids.as_mut_ptr().cast(),
            size_of_val(pids) as i32,
        )
    };
    (bytes > 0
        && (bytes as usize) < size_of_val(pids)
        && (bytes as usize).is_multiple_of(size_of::<i32>()))
    .then_some(bytes as usize / size_of::<i32>())
}
