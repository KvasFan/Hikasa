package Demo::Util;
my @list = (1, 2, 3); my $last = $#list; print scalar(@list), "\n";

=head1 NAME

Demo::Util - helpers; # not a comment here

=cut

sub greet { my ($name) = @_; return "Hi, $name\n"; } # comment
my $str = 'it\'s'; my $hex = 0xFF_FF; __END__
