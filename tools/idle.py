"""Benchmarks on a machine that other work shares.

Other processes make a run slower, never faster, and on a small machine a compiler in the
background can double a time. With `--wait-idle`, the benchmark scripts wait before each run
until the CPUs have been idle for a moment, and after a run they measure how much CPU time other
processes took during it (all the CPU time the machine spent, less the run's own); a run in which
others took more than LIMIT of one CPU is done again. What is measured, and how, does not change:
only when. The result records the largest share that others took in an accepted run.

Runs shorter than half a second are only guarded by the wait before them: the kernel counts CPU
time in ticks of 10 ms, too coarse to tell what others did in a few milliseconds.
"""

import os
import resource
import time

HZ = os.sysconf("SC_CLK_TCK")
CPUS = os.cpu_count() or 1
LIMIT = 0.15  # of one CPU
IDLE = 0.10  # of the whole machine, over the seconds before a run
TRIES = 30

enabled = False
worst = 0.0  # the largest share of a CPU others took in an accepted run
retried = 0  # runs done again because others took too much


def _jiffies():
    """(busy, total) CPU ticks of the whole machine since boot."""
    v = list(map(int, open("/proc/stat").readline().split()[1:9]))
    total = sum(v)
    return total - v[3] - v[4], total


def wait():
    """Returns once the machine has been idle (under IDLE busy) for two seconds in a row."""
    calm = 0
    while calm < 2:
        b0, t0 = _jiffies()
        time.sleep(1)
        b1, t1 = _jiffies()
        calm = calm + 1 if (b1 - b0) <= IDLE * max(1, t1 - t0) else 0


def _own():
    u = resource.getrusage(resource.RUSAGE_CHILDREN)
    return u.ru_utime + u.ru_stime


def guarded(run):
    """Calls run() (which starts and waits for a child process) and returns its result; with
    `enabled`, waits for an idle machine first and runs it again while others took more than
    LIMIT of a CPU during it."""
    global worst, retried
    if not enabled:
        return run()
    for attempt in range(TRIES):
        wait()
        b0, _ = _jiffies()
        own0, w0 = _own(), time.monotonic()
        result = run()
        b1, _ = _jiffies()
        wall = time.monotonic() - w0
        others = max(0.0, ((b1 - b0) / HZ - (_own() - own0)) / wall)
        if wall < 0.5 or others <= LIMIT or attempt == TRIES - 1:
            if wall >= 0.5:
                worst = max(worst, others)
            return result
        retried += 1


def conditions():
    """What the result should say about the machine and the guard."""
    out = {"nice": os.nice(0), "load_average_end": [round(x, 2) for x in os.getloadavg()], "wait_idle": enabled}
    if enabled:
        out.update({"others_cpu_limit": LIMIT, "others_cpu_worst": round(worst, 3), "runs_repeated": retried})
    return out
