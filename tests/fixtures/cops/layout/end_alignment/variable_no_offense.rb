# nitrocop-config: EnforcedStyleAlignWith: variable
if status
  content = label || if s = status.to_s and s.present?
                       s
                     end
end
