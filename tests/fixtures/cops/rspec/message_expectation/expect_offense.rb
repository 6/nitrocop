# nitrocop-config: EnforcedStyle: expect
allow(test_instance).to receive_messages(
^^^^^ RSpec/MessageExpectation: Prefer `expect` for setting message expectations.
  dependency_file_parser: instance_double(Foo, parse: dependencies),
  service: instance_double(Service).tap do |service|
    allow(service).to receive(:record_update_job_error)
    ^^^^^ RSpec/MessageExpectation: Prefer `expect` for setting message expectations.
  end
)
