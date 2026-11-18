#!/usr/bin/env perl
# zend-rebrand: turn Zeron's names into zend's, everywhere.
#
#   scripts/zend-rebrand.pl            rewrite the working tree (tracked files:
#                                      contents, then file and folder names)
#   scripts/zend-rebrand.pl --filter   stdin -> stdout, e.g. to cherry-pick:
#       git -C ../zeron format-patch -1 <sha> --stdout \
#         | scripts/zend-rebrand.pl --filter | git am -3
#   scripts/zend-rebrand.pl --check    list what still says zeron (exit 1 if any)
#
# Rules, in order:
#  - Repositories we build from but don't own keep their real address:
#    zeronsh/zui, zeronsh/gpui-component, zeronsh/zgx (upstream of
#    sandyhq/zengx, which apps/zgx builds against).
#  - github.com/zeronsh/ (and zeron-sh) is ours now: github.com/sandyhq/.
#  - Zeron's domains (zeron.sh and its subdomains) become *.zend.invalid
#    (RFC 6761: never resolves), never zend.sh: that name isn't ours, and a
#    client that dials it would talk to whoever owns it.
#  - Every other zeron / Zeron / ZERON becomes zend / Zend / ZEND.
use strict; use warnings;

my $SELF = "scripts/zend-rebrand.pl";

sub rebrand {
    my ($s) = @_;
    my @keep;
    $s =~ s{((?:https?://)?(?:www\.)?github\.com/zeronsh/(?:zui|gpui-component|zgx)\b|\bzeronsh/(?:zui|gpui-component|zgx)\b)}
           {push @keep, $1; "\x{1}KEEP" . $#keep . "\x{1}"}ge;
    $s =~ s{github\.com/zeron-?sh/}{github.com/sandyhq/}gi;
    $s =~ s{\bzeron-?sh\b}{sandyhq}g;
    $s =~ s{\b((?:[A-Za-z0-9-]+\.)*)zeron\.sh(?![A-Za-z0-9_])}{$1zend.invalid}gi;
    $s =~ s/ZERON/ZEND/g;
    $s =~ s/Zeron/Zend/g;
    $s =~ s/zeron/zend/g;
    # Lookalikes of the brand in the edge's name tests: zend's own.
    $s =~ s/\bzer0n\b/zen_d/g; $s =~ s/\bZer0n\b/Ze-nd/g; $s =~ s/\bzer_on\b/ze_nd/g; $s =~ s/\bz_eron\b/z_end/g;
    $s =~ s{\x{1}KEEP(\d+)\x{1}}{$keep[$1]}g;
    return $s;
}

sub rename_path {
    my ($p) = @_;
    $p =~ s/ZERON/ZEND/g; $p =~ s/Zeron/Zend/g; $p =~ s/zeron/zend/g;
    return $p;
}

my $mode = shift // "--tree";
if ($mode eq "--filter") {
    local $/; binmode STDIN; binmode STDOUT;
    my $in = <STDIN>; print rebrand($in // ""); exit 0;
}

chomp(my $root = `git rev-parse --show-toplevel`);
chdir $root or die "no repo\n";

if ($mode eq "--check") {
    my $hits = `git grep -I -n -i -e zeron -- . ':!$SELF' | grep -v -E 'zeronsh/(zui|gpui-component|zgx)'`;
    my $paths = `git ls-files | grep -i zeron`;
    print $hits, $paths;
    exit(($hits ne "" || $paths ne "") ? 1 : 0);
}
die "usage: $0 [--tree|--filter|--check]\n" unless $mode eq "--tree";

# Contents: every tracked text file that mentions it.
my @files = grep { $_ ne $SELF } split /\0/, `git grep -z -I -l -i -e zeron -- .`;
my $changed = 0;
for my $f (@files) {
    open my $in, "<:raw", $f or die "$f: $!"; local $/; my $old = <$in>; close $in;
    my $new = rebrand($old);
    next if $new eq $old;
    open my $out, ">:raw", $f or die "$f: $!"; print $out $new; close $out;
    $changed++;
}
print "contents: $changed files\n";

# Names: files whose path says it, moved to the renamed path.
my @paths = grep { rename_path($_) ne $_ } split /\0/, `git ls-files -z`;
my %seen;
for my $old (@paths) {
    my $new = rename_path($old);
    die "collision: $old -> $new exists\n" if -e $new && !$seen{$new}++;
    (my $dir = $new) =~ s{/[^/]+$}{};
    system("mkdir", "-p", $dir) == 0 or die if $dir ne $new;
    system("git", "mv", "-k", $old, $new) == 0 or die "git mv $old\n";
}
system("find . -path ./.git -prune -o -type d -empty -print | grep -i zeron | xargs -r rmdir -p 2>/dev/null");
print "paths: ", scalar(@paths), " moved\n";
