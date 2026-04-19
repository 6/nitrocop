# Variable assigned inside example
describe SomeClass do
  it 'updates the user' do
    user = create(:user)
    expect { user.update(admin: true) }.to change(user, :updated_at)
  end
end

# Variable used only as it description
describe SomeClass do
  description = "updates the user"
  it description do
    expect { user.update(admin: true) }.to change(user, :updated_at)
  end
end

# Variable used only in it description interpolation
describe SomeClass do
  article = foo ? 'a' : 'the'
  it "updates #{article} user" do
    expect { user.update(admin: true) }.to change(user, :updated_at)
  end
end

# Block parameter used in example (not reassigned)
shared_examples 'some examples' do |subject|
  it 'renders the subject' do
    expect(mail.subject).to eq(subject)
  end
end

# Block keyword parameter used in example
shared_examples 'some examples' do |subject:|
  it 'renders the subject' do
    expect(mail.subject).to eq(subject)
  end
end

# Block parameter reassigned inside example
shared_examples 'some examples' do |subject|
  it 'renders the subject' do
    subject = 'hello'
    expect(mail.subject).to eq(subject)
  end
end

# Two variables same name in different scopes
describe SomeClass do
  let(:my_user) do
    user = create(:user)
    user.flag!
    user
  end

  it 'updates the user' do
    user = create(:user)
    expect { user.update(admin: true) }.to change(user, :updated_at)
  end
end

# Variable not referenced in any example
describe SomeClass do
  user = create(:user)
  user.flag!

  it 'does something' do
    expect(foo).to eq(bar)
  end
end

# Variable used as first it_behaves_like argument (shared example name)
describe SomeClass do
  examples = foo ? 'definite article' : 'indefinite article'
  it_behaves_like examples
end

# Variable used in interpolation for it_behaves_like argument
describe SomeClass do
  article = foo ? 'a' : 'the'
  it_behaves_like 'some example', "#{article} user"
end

# Variable used in symbol interpolation for it_behaves_like argument
describe SomeClass do
  article = foo ? 'a' : 'the'
  it_behaves_like 'some example', :"#{article}_user"
end

# Block argument shadowed by local variable in iterator
describe SomeClass do
  %i[user user2].each do |user|
    let(user) do
      user = create(:user)
      user.flag!
      user
    end
  end
end

# Outside of a describe block (FactoryBot)
FactoryBot.define :foo do
  bar = 123

  after(:create) do |foo|
    foo.update(bar: bar)
  end
end

# Variable used only in skip metadata
describe SomeClass do
  skip_message = 'not yet implemented'

  it 'does something', skip: skip_message do
    expect(1 + 2).to eq(3)
  end
end

# Variable used only in pending metadata
describe SomeClass do
  pending_message = 'work in progress'

  it 'does something', pending: pending_message do
    expect(1 + 2).to eq(3)
  end
end

# Variable reassigned before use inside example (VariableForce scoping)
describe SomeClass do
  user = create(:user)

  it 'updates the user' do
    user = create(:user)
    expect { user.update(admin: true) }.to change(user, :updated_at)
  end
end

# Variable used only as first include_context argument (context name)
describe SomeClass do
  ctx = condition ? 'admin context' : 'user context'
  include_context ctx
end

# Variable used in interpolated string for include_context argument
describe SomeClass do
  role = 'admin'
  include_context 'shared setup', "#{role} context"
end

# Variable reassigned inside begin block before use (VariableForce)
describe SomeClass do
  user = create(:user)

  it 'updates the user' do
    begin
      user = create(:other_user)
      expect(user).to be_valid
    end
  end
end

# Variable used only as first arg to include_examples (the shared group name)
describe SomeClass do
  name = condition ? 'admin' : 'user'
  include_examples name
end

# Variable used only as first arg to it_should_behave_like
describe SomeClass do
  behavior = condition ? 'creates record' : 'updates record'
  it_should_behave_like behavior
end

# Variable overwritten in nested context — outer assignment dead, not used in examples
# The outer assignment's value is never read by any example scope; the variable
# is only used at group level.
describe Outer do
  config = { default: true }
  validate(config)

  context 'custom config' do
    it 'does something' do
      expect(1).to eq(1)
    end
  end
end

# Variable assigned inside iterator block param, NOT a group-level assignment
describe SomeClass do
  items.each do |item|
    item = transform(item)
    process(item)
  end

  it 'works' do
    expect(result).to eq(true)
  end
end

# Operator-assign at group level, variable NOT used in example scope
describe SomeClass do
  counter = 0
  counter += items.size

  it 'does something unrelated' do
    expect(1 + 2).to eq(3)
  end
end

# File-level variable referenced only at group level (not in example scope).
# No offense for the file-level assignment.
payload = build(:payload)

describe SomeClass do
  payload.validate  # used at group level, not in example scope

  it 'works' do
    expect(1).to eq(1)
  end
end

# File-level variable NOT referenced in any example scope — no offense.
status = :inactive

describe OtherClass do
  status  # referenced at group level only, not inside any example scope

  it 'does something' do
    expect(true).to eq(true)
  end
end

# Variable initialized at group scope, reassigned in before hook (VariableForce: dead assignment)
# RuboCop's VariableForce tracks that the before hook reassigns the variable before
# any example reads it (hooks run before examples), making the group-level value dead.
describe SomeClass do
  result = nil

  before :each do
    result = compute_something()
  end

  it 'checks the result' do
    expect(result).to eq(42)
  end
end

# Variable initialized at group scope, reassigned in before hook, read in multiple its
describe SomeClass do
  response = nil

  before do
    response = make_request()
  end

  it 'returns a response' do
    expect(response).to be_instance_of(Response)
  end

  it 'has a body' do
    expect(response.body).to eq('ok')
  end
end

# Variable initialized at group scope, reassigned in before :all hook
describe SomeClass do
  path = nil

  before :all do
    path = Dir.mktmpdir('test')
  end

  it 'uses the path' do
    expect(File.exist?(path)).to be true
  end
end

# Variable reassigned in first it block, read in second it block
# VariableForce sees linear flow: group assign -> it1 reassign -> it2 read
# and attributes the read to the it1 assignment, not the group assignment.
describe SomeClass do
  data = []

  it 'populates data' do
    data = [1, 2, 3]
  end

  it 'checks data' do
    expect(data).to eq([1, 2, 3])
  end
end

# Variable assigned inside iterator block, shadowed by block param in later iterator
# (openproject pattern: schema_name assigned in .each block, then used in a different
# .each block where schema_name is a block parameter — the block param shadows the var)
describe SomeClass do
  items.each do |item|
    schema_name = item.name
    registry[schema_name] = item
  end

  registry.each do |schema_name, item|
    describe schema_name do
      let(:schema) { load_schema(schema_name) }

      it "validates #{schema_name}" do
        expect(item).to match_schema(schema)
      end
    end
  end
end

# Variable assigned inside non-RSpec DSL method block (rswag pattern)
# post/response/path are DSL methods, not RSpec example groups or scopes.
# Variables assigned inside them and used only at the same DSL scope level
# (not inside example scopes) should not be flagged.
describe SomeClass do
  path "/api/resource" do
    post "Create resource" do
      expected_schema = load_schema("create_request")
      parameter name: :params, in: :body, schema: expected_schema

      response "200", "success" do
        expected_schema = load_schema("create_response")
        schema expected_schema

        xit
      end
    end
  end
end

# File-level variable used in non-describe-block scope (Capybara::SpecHelper.spec)
# The spec method with a receiver is NOT an RSpec example group.
# Variables assigned inside it blocks should not be collected as file-level vars.
Capybara::SpecHelper.spec '#ancestor' do
  before do
    @session.visit('/with_html')
  end

  it 'should find the element' do
    el = @session.find(:css, '#child')
    expect(el.ancestor('//p')).to have_text('Lorem ipsum')
  end

  it 'should raise on multiple matches' do
    el = @session.find(:css, '#child')
    expect { el.ancestor('//div') }.to raise_error(Capybara::Ambiguous)
  end
end

# Variable assigned inside .each at group scope, used only in example description
# (jruby pattern: format = "%" + f, used in it "supports #{format}")
describe SomeClass do
  %w(d i).each do |f|
    format = "%" + f

    it "supports integer formats using #{format}" do
      ("%#{f}" % 10).should == "10"
    end
  end
end

# Sibling block scope: same-named variable in sibling non-RSpec blocks.
# The post block has its own expected_schema that is NOT used in any example scope.
# The get block also has expected_schema that IS used in example scopes (separate offense).
# The post block's variable should NOT be flagged — it's a different local binding.
# (discourse/rswag pattern)
describe SomeClass do
  path "/api" do
    post "Create" do
      expected_schema = load_schema("create")
      parameter name: :params, schema: expected_schema
      response "200" do
        xit
      end
    end
  end
end

# Variable initialized to nil, reassigned inside nested expect block in example.
# (excon pattern: response = nil, then response = make_request() inside expect do end)
describe SomeClass do
  response = nil

  it 'returns a response' do
    expect do
      response = make_request()
    end.to_not raise_error
  end

  it 'has status' do
    expect(response.status).to eq(200)
  end
end

# Variable initialized to empty array, reassigned via lambda in example body.
# (excon pattern: data = [], then data = [...] inside lambda/block)
describe SomeClass do
  data = []
  it 'yields data' do
    response_block = lambda do |chunk, remaining, total|
      data = [chunk, remaining, total]
    end
    conn.request(response_block: response_block)
  end
  it 'has expected data' do
    expect(data).to eq(['x', 0, 100])
  end
end

# Variable inside shared_examples block (not file-level)
RSpec.shared_examples "permitted roles" do |**kwargs|
  to = kwargs.delete(:to)
  label = kwargs.except(:to).map { |key, value| "#{key} is #{value}" }.join(" AND ")

  Array(to).each do |role|
    context "#{label} #{role.inspect} authorization" do
      let(:user) { public_send(role) }
      it { is_expected.to be_truthy }
    end
  end
end

# Variable used only in describe argument (group scope, not example scope)
describe SomeClass do
  v = SomeModule::SOME_CONSTANT
  describe "with value #{v}" do
    subject { described_class.new }
    it { is_expected.to be_valid }
  end
end

# Variable used as argument to nested describe (ConstantPathNode)
# RuboCop's part_of_example_scope? doesn't match describe arguments
RSpec.describe(SomeClass) do
  result = described_class

  describe result::Success do
    it "works" do
      expect(true).to be true
    end
  end
end

# Variable used only in context metadata (group scope, not example scope)
describe SomeClass do
  exclude_test = some_platform?
  describe "platform tests", skip: exclude_test do
    it "works" do
      expect(true).to be true
    end
  end
end

# Variable used only in shared_examples_for block (not file-level)
shared_examples_for "a testable resource" do |testcase|
  context_name = "With mode #{testcase[:mode]}"
  context context_name do
    let(:resource) { build_resource(testcase) }
    it "applies correctly" do
      expect(resource).to be_valid
    end
  end
end

# Nested hash assignment used only at group scope
describe SomeClass do
  schema = {
    const: const_schema = { const: 1 }
  }

  validate(const_schema)

  it 'works' do
    expect(true).to eq(true)
  end
end

# Dead assignment: variable immediately reassigned at group scope — first
# assignment's value never reaches any example. The second assignment IS an
# offense (handled separately), but the first is dead and must not be flagged.
# (puppetlabs-docker line 43 pattern)
describe SomeClass do
  storage_driver = 'devicemapper'
  storage_driver = 'overlay2'
end

# Dead assignment inside .each block: first assignment to textile_name is
# overwritten by the second before any example scope reads it.
# (org-ruby pattern: name = join(...); name = expand(name))
describe SomeClass do
  files.each do |file|
    basename = File.basename(file, ".org")
    textile_name = File.join(data_directory, basename + ".textile")
    textile_name = File.expand_path(textile_name)
  end
end

# Dead assignment: operator-write (-=) at group scope kills previous value.
# After x = initial; x -= subset, the initial value is consumed and replaced.
# Only the final value before examples matters. (leftovers pattern)
context 'when merged' do
  merged_config_methods = ::Leftovers.config.public_methods
  merged_config_methods -= ::Class.new.new.public_methods
end

# Dead assignment: operator-write (+=) at group scope replaces value.
# (SlideHub pattern: keys = [...]; keys += [...])
describe SomeClass do
  list_json_keys = %w[id user_id name]
  list_json_keys += %w[num_of_pages created_at]
end

# Variable used only as &block argument to example method (not inside block body).
# The & reads the variable at the call site (group scope), not inside the example.
# (celluloid pattern: xit("can be deferred", &execute_deferred))
describe SomeClass do
  execute_deferred = proc do
    a1 = MyBlockActor.new("one", [])
    expect(a1.deferred_excecution(:pete) { |v| v }).to eq(:pete)
  end
  xit("can be deferred", &execute_deferred)
end

# Dead assignment: immediately overwritten by if-expression (puppetlabs pattern).
# Only the second assignment's value reaches examples; the first is dead.
describe SomeClass do
  on_supported_os.each do |os, os_facts|
    storage_driver = 'devicemapper'
    storage_driver = if os[:family] == 'RedHat'
                       'devicemapper'
                     else
                       'overlay2'
                     end
  end
end

# Dead assignment: sequential assignment within conditional (puppetlabs pattern).
# The first `facts = ...` is immediately overwritten and never reaches examples.
describe SomeClass do
  on_supported_os.each do |os, os_facts|
    if os.include?('windows')
      facts = windows_facts.merge(os_facts)
      facts = facts.merge({ puppetversion: Puppet.version })
    end
  end
end

# Lambda body in shared example args: variables inside lambdas are lambda-local
# and should not be collected as group-level assignments (natalie pattern)
describe "Kernel.sprintf" do
  it_behaves_like :kernel_sprintf_to_str, -> format, *args {
    r = nil
    -> {
    }.should_not complain(verbose: true)
    r
  }
end

# Lambda do...end body in shared example args (bugsnag pattern)
describe SomeClass do
  include_examples(
    "metadata delegate",
    lambda do |metadata, *args|
      configuration = Bugsnag::Configuration.new
      configuration.instance_variable_set(:@metadata, metadata)
      configuration.add_metadata(*args)
    end,
    lambda do |metadata, *args|
      configuration = Bugsnag::Configuration.new
      configuration.instance_variable_set(:@metadata, metadata)
    end
  )
end

# Lambda inside keyword hash arg to it_behaves_like (imap-backup pattern)
describe SomeClass do
  it_behaves_like(
    "an action that handles Logger options",
    action: ->(subject, options) do
      with_required = options.merge({"email" => "me", "server" => "host"})
      subject.invoke(:backup, [], with_required)
    end
  ) do
    let(:account) { "test" }
  end
end

# proc do...end body in shared example args (wca pattern)
describe SomeClass do
  include_examples "action",
    lambda { |current_user|
      medium = CompetitionMedium.find_by!(text: "i was just created")
      expect(medium.status).to eq "pending"
    },
    proc { |current_user|
      record = Record.find_by!(name: "test")
      expect(record).to be_valid
    }
end

# Inline assignment in it description arg (thin pattern)
describe Request, 'performance' do
  it "should be faster then #{max_parsing_time = 0.0002} RubySeconds" do
    expect { parse_request }.to be_faster_then(max_parsing_time)
  end
end

# Inline assignment in it description arg (jruby-rack pattern)
describe SomeClass do
  it spec = "still serves when retrieving exception's message fails" do
    @env[JRuby::Rack::ErrorApp::EXCEPTION] = InitException.new spec
  end
end

# Rightward pattern-match assignment (`expr => pattern`) binds locals.
# RuboCop's VariableForce declares these vars without creating assignments,
# so LeakyLocalVariable has no assignment to flag.
RSpec.describe NumberFormatHelper do
  describe "#number_with_limit" do
    [
      { input: 10, opts: {}, result: "10" }
    ].each do |test_case|
      test_case => { input:, opts:, result: }

      it "renders #{input} with #{opts} as #{result}" do
        expect(number_with_limit(input, opts)).to eq(result)
      end
    end
  end
end

