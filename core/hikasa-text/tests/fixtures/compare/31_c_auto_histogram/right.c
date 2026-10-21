#include <stdio.h>

/* Adds two numbers.
   Second line of the comment. */
int add(int a, int b)
{
    return a + b; // sum
}

int main(void)
{
    int y = add(1, 3);
    printf("%d\n", y);
    return 0;
}
