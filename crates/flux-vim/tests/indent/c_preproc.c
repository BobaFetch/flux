#ifndef HEADER_H
#define HEADER_H

#define MAX(a, b) \
    ((a) > (b) ? (a) : (b))

#ifdef DEBUG
#define LOG(x) printf("%s\n", x)
#else
#define LOG(x)
#endif

int main(void)
{
    int x = 1;
#ifdef DEBUG
    LOG("debug");
#endif
    if (x)
    {
#if 0
        never();
#endif
        x++;
    }
    return 0;
}

#endif
