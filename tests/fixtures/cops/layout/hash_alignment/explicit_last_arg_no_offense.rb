# Explicit last-argument hashes should be ignored under always_ignore and ignore_explicit.
mock_model(User,
{
  nickname: 'David',
  email: 'david@example.com',
  languages: ['Ruby'],
  skills: [],
  contributions: double(:contribution, year: []),
  suggested_projects: Project.all,
  unsubscribe_token: 'unsubscribe-token'
})

policy.attributes =
  {
    "name"        => "(Built-in) #{p_hash[:name]}",
    "description" => "(Built-in) #{p_hash[:description]}",
    :applies_to?  => p_hash[:applies_to?]
  }
