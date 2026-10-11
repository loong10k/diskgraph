/// 公共前端单次有界输入观察；来源：PF-06原连接转发合同，无Java对等对象。
/// EOF只结束输入，不表示前台输出、监督进程或原恢复资源已退休。
#[derive(Debug, PartialEq, Eq)]
pub enum FrontendRead<'a> {
    Data(&'a [u8]),
    Pending,
    Eof,
}
