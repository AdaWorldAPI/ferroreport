/* Runs a command and prints "<seconds> <peak resident KiB>" to the file named by argv[1].
 *
 * Python's own subprocess + wait4 cannot give this number for small programs: on Linux a
 * child's peak includes the memory of the process that started it, so anything lighter
 * than the Python interpreter reads as the interpreter's size. This launcher is a few
 * hundred KiB, so what it reports is the command's.
 *
 * cc -O2 -o target/maxrss tools/maxrss.c
 */
#include <stdio.h>
#include <stdlib.h>
#include <sys/resource.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

int main(int argc, char **argv) {
    if (argc < 3) return 2;
    struct timespec a, b;
    clock_gettime(CLOCK_MONOTONIC, &a);
    pid_t pid = fork();
    if (pid == 0) {
        execvp(argv[2], argv + 2);
        _exit(127);
    }
    int status = 0;
    waitpid(pid, &status, 0);
    clock_gettime(CLOCK_MONOTONIC, &b);
    struct rusage u;
    getrusage(RUSAGE_CHILDREN, &u);
    FILE *f = fopen(argv[1], "w");
    if (!f) return 2;
    fprintf(f, "%.6f %ld\n", (b.tv_sec - a.tv_sec) + (b.tv_nsec - a.tv_nsec) / 1e9, u.ru_maxrss);
    fclose(f);
    return WIFEXITED(status) ? WEXITSTATUS(status) : 1;
}
