代码审查 APPROVE，架构审查 CLEAR：原 owner 持锁期间真实 EOF、active_connections=0，同 runtime 重开四槽，再持锁 stop_and_join。失败路径释放 owner 后 join。Linux 旧实现0/3，候选3/3；macOS3/3。Windows未运行，不承诺50ms硬实时，不代表整体生产就绪。
