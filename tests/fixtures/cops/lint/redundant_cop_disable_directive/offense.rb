# Placeholder: RedundantCopDisableDirective requires post-processing
# to know which disable directives were actually needed. This cop
# is a stub that will be implemented in the linter pipeline.
x = 1
y = 2
z = 3

expect(filter.default(double(i: 0))).to be 1 # rubocop:disable RSpec/VerifiedDoubles
^ Lint/RedundantCopDisableDirective: Unnecessary disabling of `RSpec/VerifiedDoubles`.

expect(filter.default(double(i: 1))).to be 2 # rubocop:disable RSpec/VerifiedDoubles
^ Lint/RedundantCopDisableDirective: Unnecessary disabling of `RSpec/VerifiedDoubles`.

let(:value) { double(rewind: nil) } # rubocop:disable RSpec/VerifiedDoubles
^ Lint/RedundantCopDisableDirective: Unnecessary disabling of `RSpec/VerifiedDoubles`.

# rubocop:disable Style/SymbolProc
^ Lint/RedundantCopDisableDirective: Unnecessary disabling of `Style/SymbolProc`.

def mitigation_ssh_exec(command, log_stderr: false) # rubocop:disable Lint/UnusedMethodArgument
^ Lint/RedundantCopDisableDirective: Unnecessary disabling of `Lint/UnusedMethodArgument`.

Class.new(ActiveJob::Base) do # rubocop:disable Rails/ApplicationJob
^ Lint/RedundantCopDisableDirective: Unnecessary disabling of `Rails/ApplicationJob`.

Class.new(ActiveJob::Base) do # rubocop:disable Rails/ApplicationJob
^ Lint/RedundantCopDisableDirective: Unnecessary disabling of `Rails/ApplicationJob`.

Class.new(ActiveJob::Base) do # rubocop:disable Rails/ApplicationJob
^ Lint/RedundantCopDisableDirective: Unnecessary disabling of `Rails/ApplicationJob`.

def mitigation_ssh_exec(command, log_stderr: false) # rubocop:disable Lint/UnusedMethodArgument
^ Lint/RedundantCopDisableDirective: Unnecessary disabling of `Lint/UnusedMethodArgument`.

response_hash = YAML.load(response.read) # rubocop:disable Security/YAMLLoad
^ Lint/RedundantCopDisableDirective: Unnecessary disabling of `Security/YAMLLoad`.

def generate_package(old_attachment) # rubocop:disable Lint/UnusedMethodArgument
^ Lint/RedundantCopDisableDirective: Unnecessary disabling of `Lint/UnusedMethodArgument`.

def extract_meta(attachment, template_files) # rubocop:disable Lint/UnusedMethodArgument
^ Lint/RedundantCopDisableDirective: Unnecessary disabling of `Lint/UnusedMethodArgument`.

Gem::Version.new(RUBY_ENGINE_VERSION) >= '9.3.7.0' do # rubocop:disable Layout/LineLength
^ Lint/RedundantCopDisableDirective: Unnecessary disabling of `Layout/LineLength`.

Gem::Version.new(RUBY_ENGINE_VERSION) < '9.3.7.0' do # rubocop:disable Layout/LineLength
^ Lint/RedundantCopDisableDirective: Unnecessary disabling of `Layout/LineLength`.

# rubocop:disable Layout/LineLength
^ Lint/RedundantCopDisableDirective: Unnecessary disabling of `Layout/LineLength`.

# rubocop:disable Layout/LineLength
^ Lint/RedundantCopDisableDirective: Unnecessary disabling of `Layout/LineLength`.

def create_server(cloud_server) # rubocop:disable Lint/UnusedMethodArgument
^ Lint/RedundantCopDisableDirective: Unnecessary disabling of `Lint/UnusedMethodArgument`.
  raise NotImplementedError
end
