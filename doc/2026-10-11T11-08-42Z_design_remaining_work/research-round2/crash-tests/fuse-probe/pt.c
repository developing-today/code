/*
 * Trivial FUSE passthrough, used only to separate "FUSE works here" from
 * "LazyFS works here". Maps every path under MOUNT to the same path under ROOT.
 *
 * Usage: pt ROOT MOUNT
 */
#define FUSE_USE_VERSION 31

#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <fuse3/fuse.h>
#include <limits.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <unistd.h>

static const char *root;

static int under_root(char *dst, size_t n, const char *path) {
    int w = snprintf(dst, n, "%s%s", root, path);
    return (w < 0 || (size_t)w >= n) ? -ENAMETOOLONG : 0;
}

#define RESOLVE(p, path)                          \
    char p[PATH_MAX];                             \
    do {                                          \
        int rc_ = under_root(p, sizeof p, path);  \
        if (rc_) return rc_;                      \
    } while (0)

static int pt_getattr(const char *path, struct stat *st, struct fuse_file_info *fi) {
    (void)fi;
    RESOLVE(p, path);
    return lstat(p, st) == -1 ? -errno : 0;
}

static int pt_readdir(const char *path, void *buf, fuse_fill_dir_t fill, off_t off,
                      struct fuse_file_info *fi, enum fuse_readdir_flags flags) {
    (void)off; (void)fi; (void)flags;
    RESOLVE(p, path);
    DIR *d = opendir(p);
    if (!d) return -errno;
    struct dirent *de;
    while ((de = readdir(d))) {
        struct stat st;
        memset(&st, 0, sizeof st);
        st.st_ino = de->d_ino;
        st.st_mode = de->d_type << 12;
        if (fill(buf, de->d_name, &st, 0, 0)) break;
    }
    closedir(d);
    return 0;
}

static int pt_open(const char *path, struct fuse_file_info *fi) {
    RESOLVE(p, path);
    int fd = open(p, fi->flags);
    if (fd == -1) return -errno;
    fi->fh = (uint64_t)fd;
    return 0;
}

static int pt_create(const char *path, mode_t mode, struct fuse_file_info *fi) {
    RESOLVE(p, path);
    int fd = open(p, fi->flags | O_CREAT, mode);
    if (fd == -1) return -errno;
    fi->fh = (uint64_t)fd;
    return 0;
}

static int pt_read(const char *path, char *buf, size_t size, off_t off,
                   struct fuse_file_info *fi) {
    (void)path;
    ssize_t r = pread((int)fi->fh, buf, size, off);
    return r == -1 ? -errno : (int)r;
}

static int pt_write(const char *path, const char *buf, size_t size, off_t off,
                    struct fuse_file_info *fi) {
    (void)path;
    ssize_t r = pwrite((int)fi->fh, buf, size, off);
    return r == -1 ? -errno : (int)r;
}

static int pt_fsync(const char *path, int datasync, struct fuse_file_info *fi) {
    (void)path;
    int r = datasync ? fdatasync((int)fi->fh) : fsync((int)fi->fh);
    return r == -1 ? -errno : 0;
}

static int pt_release(const char *path, struct fuse_file_info *fi) {
    (void)path;
    close((int)fi->fh);
    return 0;
}

static int pt_unlink(const char *path) {
    RESOLVE(p, path);
    return unlink(p) == -1 ? -errno : 0;
}

static int pt_truncate(const char *path, off_t size, struct fuse_file_info *fi) {
    (void)fi;
    RESOLVE(p, path);
    return truncate(p, size) == -1 ? -errno : 0;
}

static const struct fuse_operations ops = {
    .getattr = pt_getattr,
    .readdir = pt_readdir,
    .open = pt_open,
    .create = pt_create,
    .read = pt_read,
    .write = pt_write,
    .fsync = pt_fsync,
    .release = pt_release,
    .unlink = pt_unlink,
    .truncate = pt_truncate,
};

int main(int argc, char *argv[]) {
    if (argc != 3) {
        fprintf(stderr, "usage: %s ROOT MOUNT\n", argv[0]);
        return 2;
    }
    root = argv[1];
    /* Foreground, single-threaded, so the caller can wait on it and kill it. */
    char *fargv[] = {argv[0], argv[2], "-f", "-s"};
    return fuse_main(4, fargv, &ops, NULL);
}
