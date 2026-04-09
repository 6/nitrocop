# nitrocop-config: EnforcedStyleAlignWith: variable
model == if other.respond_to? :model
           other.model
         else
           other
         end
         ^^^ Layout/EndAlignment: Align `end` with `if`.

def test
  field_class = case
                when :a then 1
                else 2
                end
                ^^^ Layout/EndAlignment: Align `end` with `case`.
end

def test2
  alert = case status
            when :a then 'a'
            else 'b'
          end
          ^^^ Layout/EndAlignment: Align `end` with `case`.
end
