#define _GNU_SOURCE

#include <dlfcn.h>
#include <fcntl.h>
#include <limits.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <unistd.h>

#define REDIRECT_ENV "AGENT_FLOCK_TEST_DEFAULT_LOCK_PATH"

static char default_lock_path[PATH_MAX];
static char redirected_lock_path[PATH_MAX];

__attribute__((constructor)) static void initialize_redirect(void) {
    const char *redirected = getenv(REDIRECT_ENV);
    if (redirected == NULL || redirected[0] == '\0') {
        return;
    }

    snprintf(default_lock_path, sizeof(default_lock_path), "/tmp/agent-flock-%u", geteuid());
    snprintf(redirected_lock_path, sizeof(redirected_lock_path), "%s", redirected);

    unsetenv(REDIRECT_ENV);
    unsetenv("LD_PRELOAD");
    unsetenv("DYLD_INSERT_LIBRARIES");
    unsetenv("DYLD_FORCE_FLAT_NAMESPACE");
}

static const char *redirect(const char *path) {
    if (path != NULL && default_lock_path[0] != '\0' && strcmp(path, default_lock_path) == 0) {
        return redirected_lock_path;
    }
    return path;
}

static int redirected_mkdir(const char *path, mode_t mode) {
#ifdef __APPLE__
    return mkdir(redirect(path), mode);
#else
    static int (*next_mkdir)(const char *, mode_t);
    if (next_mkdir == NULL) {
        next_mkdir = dlsym(RTLD_NEXT, "mkdir");
    }
    return next_mkdir(redirect(path), mode);
#endif
}

static int redirected_lstat(const char *path, struct stat *buffer) {
#ifdef __APPLE__
    return lstat(redirect(path), buffer);
#else
    static int (*next_lstat)(const char *, struct stat *);
    if (next_lstat == NULL) {
        next_lstat = dlsym(RTLD_NEXT, "lstat");
    }
    return next_lstat(redirect(path), buffer);
#endif
}

static int redirected_stat(const char *path, struct stat *buffer) {
#ifdef __APPLE__
    return stat(redirect(path), buffer);
#else
    static int (*next_stat)(const char *, struct stat *);
    if (next_stat == NULL) {
        next_stat = dlsym(RTLD_NEXT, "stat");
    }
    return next_stat(redirect(path), buffer);
#endif
}

static int call_open(const char *path, int flags, mode_t mode) {
#ifdef __APPLE__
    return open(redirect(path), flags, mode);
#else
    static int (*next_open)(const char *, int, ...);
    if (next_open == NULL) {
        next_open = dlsym(RTLD_NEXT, "open");
    }
    return next_open(redirect(path), flags, mode);
#endif
}

static int call_openat(int directory, const char *path, int flags, mode_t mode) {
#ifdef __APPLE__
    return openat(directory, redirect(path), flags, mode);
#else
    static int (*next_openat)(int, const char *, int, ...);
    if (next_openat == NULL) {
        next_openat = dlsym(RTLD_NEXT, "openat");
    }
    return next_openat(directory, redirect(path), flags, mode);
#endif
}

static mode_t open_mode(int flags, va_list arguments) {
    if (!(flags & O_CREAT)) {
        return 0;
    }
#ifdef __APPLE__
    return (mode_t)va_arg(arguments, int);
#else
    return va_arg(arguments, mode_t);
#endif
}

#ifdef __APPLE__

static int redirected_open(const char *path, int flags, ...) {
    va_list arguments;
    va_start(arguments, flags);
    mode_t mode = open_mode(flags, arguments);
    va_end(arguments);
    return call_open(path, flags, mode);
}

static int redirected_openat(int directory, const char *path, int flags, ...) {
    va_list arguments;
    va_start(arguments, flags);
    mode_t mode = open_mode(flags, arguments);
    va_end(arguments);
    return call_openat(directory, path, flags, mode);
}

#define DYLD_INTERPOSE(replacement, replacee)                                           \
    __attribute__((used)) static struct {                                               \
        const void *replacement;                                                        \
        const void *replacee;                                                           \
    } interpose_##replacee __attribute__((section("__DATA,__interpose"))) = {            \
        (const void *)(uintptr_t)&replacement, (const void *)(uintptr_t)&replacee       \
    }

DYLD_INTERPOSE(redirected_mkdir, mkdir);
DYLD_INTERPOSE(redirected_lstat, lstat);
DYLD_INTERPOSE(redirected_stat, stat);
DYLD_INTERPOSE(redirected_open, open);
DYLD_INTERPOSE(redirected_openat, openat);

#else

int mkdir(const char *path, mode_t mode) {
    return redirected_mkdir(path, mode);
}

int lstat(const char *path, struct stat *buffer) {
    return redirected_lstat(path, buffer);
}

int stat(const char *path, struct stat *buffer) {
    return redirected_stat(path, buffer);
}

int open(const char *path, int flags, ...) {
    va_list arguments;
    va_start(arguments, flags);
    mode_t mode = open_mode(flags, arguments);
    va_end(arguments);
    return call_open(path, flags, mode);
}

int openat(int directory, const char *path, int flags, ...) {
    va_list arguments;
    va_start(arguments, flags);
    mode_t mode = open_mode(flags, arguments);
    va_end(arguments);
    return call_openat(directory, path, flags, mode);
}

#ifdef __linux__
int lstat64(const char *path, struct stat64 *buffer) {
    static int (*next_lstat64)(const char *, struct stat64 *);
    if (next_lstat64 == NULL) {
        next_lstat64 = dlsym(RTLD_NEXT, "lstat64");
    }
    return next_lstat64(redirect(path), buffer);
}

int stat64(const char *path, struct stat64 *buffer) {
    static int (*next_stat64)(const char *, struct stat64 *);
    if (next_stat64 == NULL) {
        next_stat64 = dlsym(RTLD_NEXT, "stat64");
    }
    return next_stat64(redirect(path), buffer);
}

int statx(
    int directory,
    const char *path,
    int flags,
    unsigned int mask,
    struct statx *buffer
) {
    static int (*next_statx)(int, const char *, int, unsigned int, struct statx *);
    if (next_statx == NULL) {
        next_statx = dlsym(RTLD_NEXT, "statx");
    }
    return next_statx(directory, redirect(path), flags, mask, buffer);
}

int open64(const char *path, int flags, ...) {
    static int (*next_open64)(const char *, int, ...);
    va_list arguments;
    va_start(arguments, flags);
    mode_t mode = open_mode(flags, arguments);
    va_end(arguments);
    if (next_open64 == NULL) {
        next_open64 = dlsym(RTLD_NEXT, "open64");
    }
    return next_open64(redirect(path), flags, mode);
}
#endif

#endif
