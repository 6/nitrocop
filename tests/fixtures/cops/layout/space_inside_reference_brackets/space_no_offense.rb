# RuboCop treats multi-write index targets like `[]=` and inspects the first
# reference bracket in the target subtree, so the outer `[key]` is ignored here.
user[ 'items' ][key], other = rhs
