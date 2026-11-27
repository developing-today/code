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
// The wrapper never reports an exit code it did not read: it takes the
// default SIGCHLD disposition for itself (an inherited SIG_IGN would let
// the kernel reap the child and lose its status) and hands the child the
// disposition it inherited. If waitpid() still fails, it logs
// child-status-unknown and exits 70 instead of a made-up 0 (#15781).
//
// Known limit: a signal addressed directly to the child's pid is
// delivered only to the child, and no userspace mechanism short of
// ptrace supervision can observe its sender; ptrace would change
// the child's debugging semantics and can stall the service, so it
// is deliberately not used. Such a death is still recorded
// (child-killed, with the signal number), just without a sender.
// The wrapper is the unit's main process — the pid that systemd,
// systemctl kill, and a plain kill of the unit all address — which
// is the delivery path the 2026-11-11 actor used.
//
// Arguments are parsed strictly, per the repo's operator-script
// argument contract: the only wrapper options are --log FILE,
// --log=FILE, and the -- terminator, all before the command, and
// all validated before any side effect (no pipe, no fork, no log
// record). Any other token starting with - before the command is
// a usage error (exit 64) naming the token, so a mistyped option is
// never exec'd as a command. A command whose name starts with - is
// run after an explicit --.
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

/* Events the handler could not queue because the self-pipe was
 * full. The handler never blocks (the pipe is O_NONBLOCK): a full
 * pipe costs a receipt, never the service — the main loop reports
 * the running total so the loss is itself a matter of record. */
static volatile sig_atomic_t dropped_events = 0;

/* Deliveries not yet forwarded to the child, per signal number. The
 * handler bumps this for every caught delivery before it touches the
 * self-pipe, so forwarding never depends on the receipt getting
 * queued: a full pipe loses the receipt, never the signal. */
static volatile sig_atomic_t pending_fwd[NSIG];

static void handler(int sig, siginfo_t *info, void *uctx) {
  (void)uctx;
  if (sig > 0 && sig < NSIG) pending_fwd[sig]++;
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
  /* sizeof e is far below PIPE_BUF, so this write is atomic: it
   * either queues the whole event or, on a full pipe, fails with
   * EAGAIN and the event is counted as dropped. Never a block,
   * never a partial record. */
  if (write(selfpipe[1], &e, sizeof e) != (ssize_t)sizeof e)
    dropped_events++;
}

static const int caught[] = {SIGTERM, SIGINT, SIGHUP, SIGQUIT, SIGUSR1, SIGUSR2};

static void logline(const char *logpath, const char *fmt, ...) {
  char buf[2048];
  va_list ap;
  va_start(ap, fmt);
  vsnprintf(buf, sizeof buf, fmt, ap);
  va_end(ap);
  char line[2300];
  time_t now = time(NULL);
  struct tm tmv;
  gmtime_r(&now, &tmv);
  char ts[32];
  strftime(ts, sizeof ts, "%Y-%m-%dT%H:%M:%SZ", &tmv);
  int n = snprintf(line, sizeof line, "%s %s\n", ts, buf);
  if (n < 0) return;
  if ((size_t)n >= sizeof line) n = sizeof line - 1;
  int fd = open(logpath, O_WRONLY | O_CREAT | O_APPEND, 0644);
  if (fd < 0) {
    fprintf(stderr, "sigterm-witness: cannot open log %s (%s); receipt: %s",
            logpath, strerror(errno), line);
    return;
  }
  ssize_t r = write(fd, line, (size_t)n);
  if (r != (ssize_t)n)
    fprintf(stderr, "sigterm-witness: cannot write log %s (%s); receipt: %s",
            logpath, strerror(errno), line);
  close(fd);
}

/* Forward every delivery counted since the last call. The caught
 * signals are blocked while the counters are taken, so a delivery
 * landing mid-update is neither lost nor counted twice. */
static void forward_pending(pid_t child) {
  sigset_t caught_set, old;
  sigemptyset(&caught_set);
  for (size_t k = 0; k < sizeof caught / sizeof caught[0]; k++)
    sigaddset(&caught_set, caught[k]);
  sigprocmask(SIG_BLOCK, &caught_set, &old);
  int take[sizeof caught / sizeof caught[0]];
  for (size_t k = 0; k < sizeof caught / sizeof caught[0]; k++) {
    take[k] = pending_fwd[caught[k]];
    pending_fwd[caught[k]] = 0;
  }
  sigprocmask(SIG_SETMASK, &old, NULL);
  for (size_t k = 0; k < sizeof caught / sizeof caught[0]; k++)
    for (int j = 0; j < take[k]; j++)
      kill(child, caught[k]);
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
  /* Strict option-prefix parse (operator-script argument
   * contract): --log FILE, --log=FILE, and -- are the only wrapper
   * options, and every one is validated here — before the pipe,
   * the fork, and any log record. An unknown option is a usage
   * error naming the option, never a command name to exec. */
  while (i < argc) {
    if (strcmp(argv[i], "--") == 0) { i++; break; }
    if (strcmp(argv[i], "--log") == 0) {
      if (i + 1 < argc && argv[i + 1][0] == '\0') {
        fprintf(stderr, "sigterm-witness: --log requires a non-empty FILE\n"
                        "usage: sigterm-witness --log FILE -- CMD [ARGS...]\n");
        return 64;
      }
      if (i + 1 >= argc) {
        fprintf(stderr, "sigterm-witness: --log requires a FILE argument\n"
                        "usage: sigterm-witness --log FILE -- CMD [ARGS...]\n");
        return 64;
      }
      logpath = argv[i + 1];
      i += 2;
      continue;
    }
    if (strncmp(argv[i], "--log=", 6) == 0) {
      if (argv[i][6] == '\0') {
        fprintf(stderr, "sigterm-witness: --log= requires a non-empty FILE\n"
                        "usage: sigterm-witness --log FILE -- CMD [ARGS...]\n");
        return 64;
      }
      logpath = argv[i] + 6;
      i++;
      continue;
    }
    if (argv[i][0] == '-') {
      fprintf(stderr, "sigterm-witness: unknown option %s\n"
                      "usage: sigterm-witness --log FILE -- CMD [ARGS...]\n",
              argv[i]);
      return 64;
    }
    break; /* first non-option token: the command */
  }
  if (i >= argc || logpath == NULL) {
    fprintf(stderr, "usage: sigterm-witness --log FILE -- CMD [ARGS...]\n");
    return 64;
  }

  if (pipe2(selfpipe, O_CLOEXEC | O_NONBLOCK) != 0) { perror("pipe2"); return 70; }

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

  /* #15781: an inherited SIGCHLD of SIG_IGN (or SA_NOCLDWAIT) makes the
   * kernel reap the child itself, so waitpid() fails with ECHILD and the
   * exit status is lost. Take the default disposition for the wrapper,
   * and give the child back what it would have inherited without us. */
  struct sigaction chld_dfl, chld_orig;
  memset(&chld_dfl, 0, sizeof chld_dfl);
  chld_dfl.sa_handler = SIG_DFL;
  sigemptyset(&chld_dfl.sa_mask);
  if (sigaction(SIGCHLD, &chld_dfl, &chld_orig) != 0) { perror("sigaction"); return 70; }

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
    sigaction(SIGCHLD, &chld_orig, NULL);
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
  int wait_errno = 0; /* non-zero: waitpid failed and status is unknown */
  sig_atomic_t dropped_reported = 0;
  char rbuf[sizeof(struct evt) * 8];
  while (!child_done) {
    /* Drain any pending signal events (non-blocking read). */
    forward_pending(child); /* the service must feel every delivery */
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
      }
    }
    if (dropped_events != dropped_reported) {
      logline(logpath,
              "signal-events-dropped count=%d (self-pipe full; the handler never blocks)",
              (int)dropped_events);
      dropped_reported = dropped_events;
    }
    pid_t w = waitpid(child, &status, WNOHANG);
    if (w == child) { child_done = 1; break; }
    if (w < 0 && errno != EINTR) { wait_errno = errno; break; }
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

  if (wait_errno != 0) {
    /* Never report a code we did not read: a guessed 0 would make a
     * failed service look clean to its supervisor (#15781). */
    logline(logpath, "child-status-unknown reason=\"%s\" child_pid=%d",
            strerror(wait_errno), (int)child);
    return 70;
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
