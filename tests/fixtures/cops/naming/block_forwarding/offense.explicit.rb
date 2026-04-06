# anonymous block forwarding in regular method
def foo(&)
        ^ Naming/BlockForwarding: Use explicit block forwarding.
  bar(&)
      ^ Naming/BlockForwarding: Use explicit block forwarding.
end
# anonymous block with multiple forwarding
def multi(&)
          ^ Naming/BlockForwarding: Use explicit block forwarding.
  bar(&)
      ^ Naming/BlockForwarding: Use explicit block forwarding.
  baz(qux, &)
           ^ Naming/BlockForwarding: Use explicit block forwarding.
end
# anonymous block in singleton method
def self.singleton_fwd(&)
                       ^ Naming/BlockForwarding: Use explicit block forwarding.
  bar(&)
      ^ Naming/BlockForwarding: Use explicit block forwarding.
end
# anonymous block with yield
def with_yield(&)
               ^ Naming/BlockForwarding: Use explicit block forwarding.
  yield
end
# anonymous block without body
def empty_body(&)
               ^ Naming/BlockForwarding: Use explicit block forwarding.
end
# anonymous block with symbol proc in body
def with_symbol_proc(&)
                     ^ Naming/BlockForwarding: Use explicit block forwarding.
  bar(&:do_something)
end
# anonymous block with keyword params (still flagged for explicit style)
def with_kwarg(k:, &)
                   ^ Naming/BlockForwarding: Use explicit block forwarding.
  bar(&)
      ^ Naming/BlockForwarding: Use explicit block forwarding.
end
# anonymous block nested inside a non-anonymous block pass expression
def complete(&)
             ^ Naming/BlockForwarding: Use explicit block forwarding.
  provider.complete(messages, &wrap_streaming_block(&))
                                                    ^ Naming/BlockForwarding: Use explicit block forwarding.
  handle_tool_calls(response, &)
                              ^ Naming/BlockForwarding: Use explicit block forwarding.
end
# block forwarding name already in use — still flag but no autocorrect
def name_conflict(block, &)
                         ^ Naming/BlockForwarding: Use explicit block forwarding.
  bar(block, &)
             ^ Naming/BlockForwarding: Use explicit block forwarding.
end
