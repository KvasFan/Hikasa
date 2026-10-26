#!/usr/bin/perl
use strict;
use warnings;

# Count the words of a file.
my %count;
open(my $fh, '<', $ARGV[0]) or die "cannot open: $!";
while (my $line = <$fh>) {
    chomp $line;
    $count{lc $_}++ for split /\s+/, $line;
}
close $fh;
printf "%s %d\n", $_, $count{$_} for sort keys %count;
