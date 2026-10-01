#include <stdio.h>
#include "local.h"

static int counter = 0;
unsigned long total,
              other;

int
add(a, b)
int a;
int b;
{
    return a + b;
}

static const char *
name_of(int kind)
{
    return kind ? "one" : "two";
}

int long_call(int first, int second,
              int third)
{
    int value = compute(first,
                        second,
                        third);
    int sum = first +
        second +
        third;
    printf("%d %d\n",
           value, sum);
    if (first &&
        second)
        return 1;
    return function_with_a_long_name(
        first, second);
}
