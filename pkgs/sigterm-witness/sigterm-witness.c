// sigterm-witness — wrap a service's real command and keep a signed
// receipt for every signal delivered to the wrapper.
//
// Why (code#8777): on 2026-11-11 an unidentified actor sent SIGTERM to
// pm-aggregator and pm-dispatch in the same second. systemd recorded
// the deaths (code=killed, signal=TERM) but nothing anywhere recorded
// WHO sent the signal — the laptop has no auditd. A signal handler
// installed with SA_SIGINFO receives the sender's pid and uid from the
// kernel (siginfo_t.si_pid / si_uid) at delivery time; this wrapper
// logs them (plus the sender's /proc cmdline, read immediately) and
// then forwards the signal to the real process, so the service behaves
// exactly as if it had received the signal directly.
//
// The wrapper is transparent: it forwards every caught signal to the
// child, exits with the child's exit code, and if the child dies by a
// signal it re-raises that signal on itself so the supervisor sees the
// identical death (code=killed, signal=N) it would have seen unwrapped.
//
// Usage: sigterm-witness --log FILE -- CMD [ARGS...]
//        (log path may also come from SIGTERM_WITNESS_LOG)

#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

struct evt {
  int sig;
  pid_t sender_pid;
  uid_t sender_uid;
  int si_code;
  pid_t sender_ppid;
  unsigned long long sender_starttime; /* /proc/<pid>/stat field 22 */
  int cmdlen;
  char cmdline[512]; /* raw NUL-separated, snapshotted in the handler */
  char sender_comm[64]; /* /proc/<pid>/stat comm — survives zombification */
};

static int selfpipe[2];

static void handler(int sig, siginfo_t *info, void *uctx) {
  (void)uctx;
  struct evt e;
  e.sig = sig;
  e.sender_pid = info ? info->si_pid : -1;
  e.sender_uid = info ? info->si_uid : (uid_t)-1;
  e.si_code = info ? info->si_code : 0;
  e.sender_ppid = -1;
  e.sender_starttime = 0;
  e.cmdlen = 0;
  e.cmdline[0] = '\0';
  e.sender_comm[0] = '\0';
  /* Snapshot the sender's identity NOW, in the handler: a pkill-style
   * sender exits microseconds after sending, so a snapshot taken even
   * 20ms later in the main loop races the sender's exit. Two artifacts:
   * cmdline (complete while the sender lives; EMPTY once it is a
   * zombie) and stat (comm + ppid + starttime stay readable in a
   * zombie; the ppid usually names a surviving parent — a session
   * shell, a supervisor, systemd — whose own cmdline the post-mortem
   * can still read). open/read/close are async-signal-safe, and
   * sa_mask (set at install) blocks the other caught signals for the
   * duration, so these bounded reads cannot be re-entered. */
  if (e.sender_pid > 0) {
    char base[48];
    int bl = 0;
    const char *pfx = "/proc/";
    while (pfx[bl]) { base[bl] = pfx[bl]; bl++; }
    char digits[16];
    int nd = 0;
    long v = (long)e.sender_pid;
    if (v == 0) digits[nd++] = '0';
    while (v > 0) { digits[nd++] = (char)('0' + (v % 10)); v /= 10; }
    while (nd > 0) base[bl++] = digits[--nd];
    base[bl] = '\0';

    char path[64];
    int pl = 0;
    while (base[pl]) { path[pl] = base[pl]; pl++; }
    const char *sfx = "/cmdline";
    int si = 0;
    while (sfx[si]) { path[pl++] = sfx[si]; si++; }
    path[pl] = '\0';
    int fd = open(path, O_RDONLY);
    if (fd >= 0) {
      ssize_t n = read(fd, e.cmdline, sizeof e.cmdline - 1);
      close(fd);
      if (n > 0) { e.cmdlen = (int)n; e.cmdline[n] = '\0'; }
    }

    pl = 0;
    while (base[pl]) { path[pl] = base[pl]; pl++; }
    sfx = "/stat";
    si = 0;
    while (sfx[si]) { path[pl++] = sfx[si]; si++; }
    path[pl] = '\0';
    fd = open(path, O_RDONLY);
    if (fd >= 0) {
      char sb[1024];
      ssize_t n = read(fd, sb, sizeof sb - 1);
      close(fd);
      if (n > 0) {
        sb[n] = '\0';
        /* comm sits between the first '(' and the last ')'. */
        int lp = -1, rp = -1;
        for (int k = 0; k < n; k++) {
          if (sb[k] == '(' && lp < 0) lp = k;
          if (sb[k] == ')') rp = k;
        }
        if (lp >= 0 && rp > lp) {
          int cl = rp - lp - 1;
          if (cl > (int)sizeof e.sender_comm - 1) cl = (int)sizeof e.sender_comm - 1;
          for (int k = 0; k < cl; k++) e.sender_comm[k] = sb[lp + 1 + k];
          e.sender_comm[cl] = '\0';
          /* Tokens after ") ": field 3 (state) is token 0, so ppid
           * (field 4) is token 1 and starttime (field 22) token 19. */
          int tok = 0;
          long ppid = -1;
          unsigned long long start = 0;
          int k = rp + 2;
          while (k < n && tok <= 19) {
            long num = 0;
            int any = 0;
            while (k < n && sb[k] >= '0' && sb[k] <= '9') { num = num * 10 + (sb[k] - '0'); any = 1; k++; }
            if (any) {
              if (tok == 1) ppid = num;
              if (tok == 19) start = (unsigned long long)num;
            }
            while (k < n && sb[k] != ' ') k++;
            if (k < n) k++;
            tok++;
          }
          e.sender_ppid = (pid_t)ppid;
          e.sender_starttime = start;
        }
      }
    }
  }
  ssize_t r = write(selfpipe[1], &e, sizeof e);
  (void)r;
}

static const int caught[] = {SIGTERM, SIGINT, SIGHUP, SIGQUIT, SIGUSR1, SIGUSR2};

static void logline(const char *logpath, const char *fmt, ...) {
  char buf[2048];
  va_list ap;
  va_start(ap, fmt);
  vsnprintf(buf, sizeof buf, fmt, ap);
  va_end(ap);
  int fd = open(logpath, O_WRONLY | O_CREAT | O_APPEND, 0644);
  if (fd < 0) return;
  char line[2300];
  time_t now = time(NULL);
  struct tm tmv;
  gmtime_r(&now, &tmv);
  char ts[32];
  strftime(ts, sizeof ts, "%Y-%m-%dT%H:%M:%SZ", &tmv);
  int n = snprintf(line, sizeof line, "%s %s\n", ts, buf);
  ssize_t r = write(fd, line, (size_t)n);
  (void)r;
  close(fd);
}

/* Render the handler-snapshotted raw cmdline: NULs -> spaces. Empty
 * when the sender had already exited before delivery (si_pid/uid and
 * the timestamp remain the durable facts in that case). */
static void fmt_cmdline(const struct evt *e, char *out, size_t outsz) {
  size_t n = 0;
  if (e->cmdlen > 0) {
    n = (size_t)e->cmdlen < outsz - 1 ? (size_t)e->cmdlen : outsz - 1;
    memcpy(out, e->cmdline, n);
  }
  out[n] = '\0';
  for (size_t i = 0; i < n; i++) if (out[i] == '\0') out[i] = ' ';
  while (n > 0 && out[n - 1] == ' ') out[--n] = '\0';
}

static const char *signame(int sig) {
  switch (sig) {
    case SIGTERM: return "SIGTERM";
    case SIGINT: return "SIGINT";
    case SIGHUP: return "SIGHUP";
    case SIGQUIT: return "SIGQUIT";
    case SIGUSR1: return "SIGUSR1";
    case SIGUSR2: return "SIGUSR2";
    default: return "SIG?";
  }
}

int main(int argc, char **argv) {
  const char *logpath = getenv("SIGTERM_WITNESS_LOG");
  int i = 1;
  if (i + 1 < argc && strcmp(argv[i], "--log") == 0) {
    logpath = argv[i + 1];
    i += 2;
  }
  if (i < argc && strcmp(argv[i], "--") == 0) i++;
  if (i >= argc || logpath == NULL) {
    fprintf(stderr, "usage: sigterm-witness --log FILE -- CMD [ARGS...]\n");
    return 64;
  }

  if (pipe2(selfpipe, O_CLOEXEC) != 0) { perror("pipe2"); return 70; }

  struct sigaction sa;
  memset(&sa, 0, sizeof sa);
  sa.sa_sigaction = handler;
  sa.sa_flags = SA_SIGINFO;
  sigemptyset(&sa.sa_mask);
  /* Block all caught signals while one handler runs: the handler's
   * /proc snapshot must not be re-entered by a second signal. */
  for (size_t k = 0; k < sizeof caught / sizeof caught[0]; k++)
    sigaddset(&sa.sa_mask, caught[k]);
  for (size_t k = 0; k < sizeof caught / sizeof caught[0]; k++)
    sigaction(caught[k], &sa, NULL);

  pid_t child = fork();
  if (child < 0) { perror("fork"); return 70; }
  if (child == 0) {
    /* Real process: default dispositions, just exec. */
    struct sigaction dfl;
    memset(&dfl, 0, sizeof dfl);
    dfl.sa_handler = SIG_DFL;
    sigemptyset(&dfl.sa_mask);
    for (size_t k = 0; k < sizeof caught / sizeof caught[0]; k++)
      sigaction(caught[k], &dfl, NULL);
    close(selfpipe[0]); close(selfpipe[1]);
    execvp(argv[i], &argv[i]);
    _exit(127);
  }

  /* NOTE: the parent must keep selfpipe[1] open — the signal handler
   * (which runs in the parent) writes events to it. Closing it here
   * would silently swallow every caught signal. */
  logline(logpath, "witness-start witness_pid=%d child_pid=%d cmd=%s",
          (int)getpid(), (int)child, argv[i]);

  int status = 0;
  int child_done = 0;
  char rbuf[sizeof(struct evt) * 8];
  while (!child_done) {
    /* Drain any pending signal events (non-blocking read). */
    fcntl(selfpipe[0], F_SETFL, O_NONBLOCK);
    ssize_t n;
    while ((n = read(selfpipe[0], rbuf, sizeof rbuf)) > 0) {
      for (ssize_t off = 0; off + (ssize_t)sizeof(struct evt) <= n; off += sizeof(struct evt)) {
        struct evt e;
        memcpy(&e, rbuf + off, sizeof e);
        char cmd[600];
        fmt_cmdline(&e, cmd, sizeof cmd);
        logline(logpath,
                "signal=%s(%d) sender_pid=%d sender_uid=%d si_code=%d sender_comm=\"%s\" sender_ppid=%d sender_starttime=%llu sender_cmdline=\"%s\" child_pid=%d",
                signame(e.sig), e.sig, (int)e.sender_pid, (int)e.sender_uid,
                e.si_code, e.sender_comm, (int)e.sender_ppid,
                e.sender_starttime, cmd, (int)child);
        kill(child, e.sig); /* forward — the service must feel it */
      }
    }
    pid_t w = waitpid(child, &status, WNOHANG);
    if (w == child) { child_done = 1; break; }
    if (w < 0 && errno != EINTR) break;
    struct timespec ts = {0, 20 * 1000 * 1000}; /* 20ms */
    nanosleep(&ts, NULL);
  }
  /* Final drain: a signal that raced the child's death still counts. */
  fcntl(selfpipe[0], F_SETFL, O_NONBLOCK);
  ssize_t n;
  while ((n = read(selfpipe[0], rbuf, sizeof rbuf)) > 0) {
    for (ssize_t off = 0; off + (ssize_t)sizeof(struct evt) <= n; off += sizeof(struct evt)) {
      struct evt e;
      memcpy(&e, rbuf + off, sizeof e);
      char cmd[600];
      fmt_cmdline(&e, cmd, sizeof cmd);
      logline(logpath,
              "signal=%s(%d) sender_pid=%d sender_uid=%d si_code=%d sender_comm=\"%s\" sender_ppid=%d sender_starttime=%llu sender_cmdline=\"%s\" child_pid=%d (after child exit)",
              signame(e.sig), e.sig, (int)e.sender_pid, (int)e.sender_uid,
              e.si_code, e.sender_comm, (int)e.sender_ppid,
              e.sender_starttime, cmd, (int)child);
    }
  }

  if (WIFEXITED(status)) {
    logline(logpath, "child-exit code=%d child_pid=%d", WEXITSTATUS(status), (int)child);
    return WEXITSTATUS(status);
  }
  if (WIFSIGNALED(status)) {
    int s = WTERMSIG(status);
    logline(logpath, "child-killed signal=%s(%d) child_pid=%d", signame(s), s, (int)child);
    /* Die the same death so the supervisor's record is unchanged. */
    struct sigaction dfl;
    memset(&dfl, 0, sizeof dfl);
    dfl.sa_handler = SIG_DFL;
    sigemptyset(&dfl.sa_mask);
    sigaction(s, &dfl, NULL);
    raise(s);
    _exit(128 + s);
  }
  return 70;
}
