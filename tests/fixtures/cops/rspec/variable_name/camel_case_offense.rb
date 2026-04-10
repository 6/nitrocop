# nitrocop-config: EnforcedStyle: camelCase
RSpec.describe User do
  let(:Authorization) { "Bearer #{user.api_key}" }
      ^^^^^^^^^^^^^^ RSpec/VariableName: Use camelCase for variable names.
end
