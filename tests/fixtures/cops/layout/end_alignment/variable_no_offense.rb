# nitrocop-config: EnforcedStyleAlignWith: variable
if status
  content = label || if s = status.to_s and s.present?
                       s
                     end
end

def test
  !! case destroy_option
     when Symbol, String
       1
     else
       2
     end
end

def test2
  field_class = case
  when :a then 1
  else 2
  end
end

def test3
  alert = case status
            when :a then 'a'
            else 'b'
  end
end
