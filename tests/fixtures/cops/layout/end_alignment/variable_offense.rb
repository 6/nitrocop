# nitrocop-config: EnforcedStyleAlignWith: variable
model == if other.respond_to? :model
           other.model
         else
           other
         end
         ^^^ Layout/EndAlignment: Align `end` with `if`.
