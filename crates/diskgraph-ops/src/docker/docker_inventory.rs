//! docker_inventory：既有文件操作职责的原生 Rust 实现。
use crate::docker::docker_object::DockerObject;

/// 分门记录 Docker 对象清单与虚拟机空间说明。
/// 来源：DiskGraph 原生 Rust `diskgraph_ops::docker::DockerInventory`，保留既有语义。
/// The read-only inventory (7.8). Bytes per category are separate: caches,
/// images, containers, and volumes answer different questions, and merging
/// them would overstate what any one cleanup could free.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DockerInventory {
    pub build_cache: Vec<DockerObject>,
    pub images: Vec<DockerObject>,
    pub containers: Vec<DockerObject>,
    pub volumes: Vec<DockerObject>,
    /// Measured limits and caveats the caller must see.
    pub notes: Vec<String>,
}

impl DockerInventory {
    /// 按原有分类顺序借用所有 Docker 对象。
    /// 参数：self 为分类清单。
    /// 返回：构建缓存、镜像、容器、卷顺序的对象引用列表。
    /// Every object, whatever its category.
    pub fn all(&self) -> Vec<&DockerObject> {
        self.build_cache
            .iter()
            .chain(self.images.iter())
            .chain(self.containers.iter())
            .chain(self.volumes.iter())
            .collect()
    }
}
