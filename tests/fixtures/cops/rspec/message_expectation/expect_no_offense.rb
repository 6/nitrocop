# nitrocop-config: EnforcedStyle: expect
allow(test_instance).to receive_messages(
  dependency_file_parser: instance_double(Foo, parse: dependencies),
  service: instance_double(Service)
)
