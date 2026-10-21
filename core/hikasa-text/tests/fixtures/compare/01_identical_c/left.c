#include <stdio.h>

/* Adds two numbers. */
int add(int a, int b)
{
    return a + b;
}

int main(void)
{
    int x = add(1, 2);
    printf("%d\n", x);
    return 0;
}
