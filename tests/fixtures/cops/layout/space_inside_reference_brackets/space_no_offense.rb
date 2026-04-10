# RuboCop treats multi-write index targets like `[]=` and inspects the first
# reference bracket in the target subtree, so the outer `[key]` is ignored here.
user[ 'items' ][key], other = rhs

# Multiline receivers still make RuboCop skip `[]` reads, but the same
# pattern is an offense once the receiver fits on one line.
Bookmark.tag_counts.sort { |a, b|
  a.count <=> b.count
}.reverse[0..49]
