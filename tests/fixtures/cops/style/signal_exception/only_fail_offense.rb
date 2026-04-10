# nitrocop-config: EnforcedStyle: only_fail

Kernel.raise "error"
       ^^^^^ Style/SignalException: Use `fail` instead of `raise` to rethrow exceptions.

::Kernel.raise "error"
         ^^^^^ Style/SignalException: Use `fail` instead of `raise` to rethrow exceptions.
