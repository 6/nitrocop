obj.then(&:test)
    ^^^^ Style/ObjectThen: Prefer `yield_self` over `then`.

self.then(actor) { |v, a| a.ask_op(v) }
     ^^^^ Style/ObjectThen: Prefer `yield_self` over `then`.
