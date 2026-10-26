#include <stdio.h>
#include "list.h"

/* Sum the values of a list.
 * Returns 0 for an empty list.
 */
static long sum(const struct node *head)
{
    long total = 0L;
    for (const struct node *n = head; n != NULL; n = n->next) {
        total += n->value; // accumulate
    }
    return total;
}

int main(void)
{
    printf("total = %ld\n", sum(NULL));
    return 0;
}
