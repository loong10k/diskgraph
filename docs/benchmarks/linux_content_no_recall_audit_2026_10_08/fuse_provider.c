// 隔离 Linux 原生 FUSE 提供方：只有 placeholder 对象，READ 回调记录真实内容请求。
#include <linux/fuse.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <unistd.h>
#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
static const char content[] = "synthetic-cloud-content";
static struct fuse_attr attributes(uint64_t node) {
    struct fuse_attr a = {0};
    a.ino=node; a.size=node==1?4096:sizeof(content)-1; a.blocks=1;
    a.atime=a.mtime=a.ctime=1; a.mode=node==1?(S_IFDIR|0755):(S_IFREG|0644);
    a.nlink=node==1?2:1; a.uid=getuid(); a.gid=getgid(); a.blksize=4096;
    return a;
}
static int reply(int fd,uint64_t unique,int error,const void *data,size_t size) {
    unsigned char bytes[8192];
    struct fuse_out_header h={.len=(uint32_t)(sizeof(h)+size),.error=-error,.unique=unique};
    if(sizeof(h)+size>sizeof(bytes)) return -1;
    memcpy(bytes,&h,sizeof(h)); if(size) memcpy(bytes+sizeof(h),data,size);
    return write(fd,bytes,sizeof(h)+size)==(ssize_t)(sizeof(h)+size)?0:-1;
}
int main(int argc,char **argv) {
    if(argc!=2) return 2;
    int fd=open("/dev/fuse",O_RDWR|O_CLOEXEC); if(fd<0){perror("open fuse");return 3;}
    char options[128];snprintf(options,sizeof(options),"fd=%d,rootmode=40000,user_id=%u,group_id=%u,max_read=4096",fd,getuid(),getgid());
    if(mount("isolated-no-recall-provider",argv[1],"fuse",MS_NOSUID|MS_NODEV,options)){perror("mount fuse");close(fd);return 4;}
    printf("PROVIDER_MOUNTED\n");fflush(stdout);unsigned reads=0;
    for(;;){
        unsigned char input[8192];ssize_t got=read(fd,input,sizeof(input));
        if(got<0 && errno==EINTR) continue;
        if(got<0 && errno==ENODEV) break;
        if(got<(ssize_t)sizeof(struct fuse_in_header)){perror("fuse request");close(fd);return 5;}
        struct fuse_in_header *h=(struct fuse_in_header *)input;int error=0;int result=0;
        switch(h->opcode){
        case FUSE_INIT:{struct fuse_init_out o={0};o.major=7;o.minor=31;o.max_write=4096;o.time_gran=1;result=reply(fd,h->unique,0,&o,sizeof(o));break;}
        case FUSE_LOOKUP:{const char *name=(char *)(input+sizeof(*h));if(h->nodeid!=1 || strcmp(name,"placeholder")){error=ENOENT;break;}struct fuse_entry_out o={0};o.nodeid=2;o.generation=1;o.entry_valid=o.attr_valid=1;o.attr=attributes(2);result=reply(fd,h->unique,0,&o,sizeof(o));break;}
        case FUSE_GETATTR:{struct fuse_attr_out o={0};o.attr_valid=1;o.attr=attributes(h->nodeid);result=reply(fd,h->unique,0,&o,sizeof(o));break;}
        case FUSE_STATFS:{struct fuse_statfs_out o={0};o.st.blocks=o.st.bfree=1;o.st.files=2;o.st.ffree=1;o.st.bsize=o.st.frsize=4096;o.st.namelen=255;result=reply(fd,h->unique,0,&o,sizeof(o));break;}
        case FUSE_OPEN:printf("FUSE_DATA_OPEN\n");fflush(stdout); /* fall through */
        case FUSE_OPENDIR:{struct fuse_open_out o={0};o.fh=h->nodeid;o.open_flags=FOPEN_DIRECT_IO;result=reply(fd,h->unique,0,&o,sizeof(o));break;}
        case FUSE_READ:{struct fuse_read_in *r=(struct fuse_read_in *)(input+sizeof(*h));size_t n=sizeof(content)-1;if(r->offset>=n)n=0;else n-=r->offset;if(n>r->size)n=r->size;if(n){reads++;printf("FUSE_FETCH_DATA=%u\n",reads);fflush(stdout);}result=reply(fd,h->unique,0,n?content+r->offset:NULL,n);break;}
        case FUSE_RELEASE:case FUSE_RELEASEDIR:case FUSE_FLUSH:result=reply(fd,h->unique,0,NULL,0);break;
        case FUSE_FORGET:case FUSE_BATCH_FORGET:continue;
        case FUSE_DESTROY:close(fd);return 0;
        default:error=ENOSYS;break;
        }
        if(error) result=reply(fd,h->unique,error,NULL,0);
        if(result){perror("fuse reply");close(fd);return 6;}
    }
    close(fd);return 0;
}
