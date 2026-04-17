# nitrocop-config: EnforcedStyle: aggressive
it 'flags uniq nested inside a single block-body statement' do
  expect(recs.pluck(:provider).uniq).to eq(['greenlight'])
                               ^^^^ Rails/UniqBeforePluck: Use `distinct` before `pluck`.
end

it 'flags indexed uniq nested inside a single block-body statement' do
  expect(user_backup.user_second_factors.backup_codes.pluck(:method).uniq[0]).to eq(method)
                                                                     ^^^^ Rails/UniqBeforePluck: Use `distinct` before `pluck`.
end
