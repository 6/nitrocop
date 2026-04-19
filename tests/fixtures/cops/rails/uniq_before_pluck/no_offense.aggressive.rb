# nitrocop-config: EnforcedStyle: aggressive
cache { relation.pluck(:name).uniq }
cache do
  relation.pluck(:name).uniq
end
