#include <stdio.h>
#define MAX 10

/* A sample for comparing highlights. */
typedef struct point {
    int x, y;
} point_t;

static const char *name = "flux\n";

int main(int argc, char **argv) {
    for (int i = 0; i < MAX; i++) {
        if (i % 2 == 0 && argc > 1) {
            printf("%d %s\n", i, argv[1]);
        }
    }
    return sizeof(point_t) > 0 ? 0 : 1; // done
}
