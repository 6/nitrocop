# Chained multiline hash should report on `.and_return`, not the root receiver
it do
  allow(controller).to receive(:repo_with_labels)
    .with('24pullrequests/24pullrequests')
    .and_return({
     ^^^^^^^^^^ RSpec/ReturnFromStub: Use block for static values.
      data: {
        repository: { html_url: "/foo" },
        labels: ['foo', 'bar']
      },
      status: 200
    })
end

# Keyword args are static values for block style too
it do
  allow(i).to receive(:settings).and_return(
                                 ^^^^^^^^^^ RSpec/ReturnFromStub: Use block for static values.
    multiple: true,
    select_values: ["handhelds", "fridges", "watches"]
  )
end

# Parenthesized literals after a spaced `and_return` are still static
it do
  expect(@tester).to receive(:render)
    .with("person_fields", { f: nil, dynamic: true, alfred: "Judoka" })
    .and_return ("partiallll")
     ^^^^^^^^^^ RSpec/ReturnFromStub: Use block for static values.
end

# Multiline `.with(...)` chains still flag when the static value is parenthesized
it do
  allow(Puppet::Util::Windows::User).to receive(:logon_user_by_logon_type).with(
    user, ".", passwd, fLOGON32_LOGON_NETWORK, fLOGON32_PROVIDER_DEFAULT, anything
  ).and_return (0)
    ^^^^^^^^^^ RSpec/ReturnFromStub: Use block for static values.
end

# Interpolated strings with only constants are still static values
it do
  allow(service).to receive(:url).and_return("#{AWS_S3_ENDPOINT}/#{S3_BUCKET}/test-url")
                                  ^^^^^^^^^^ RSpec/ReturnFromStub: Use block for static values.
end

# Adjacent string literals are still static values
it do
  allow(IO).to receive(:read).and_return(
                              ^^^^^^^^^^ RSpec/ReturnFromStub: Use block for static values.
    "cucumber.feature:1:3\n" \
    "cucumber.feature:5 cucumber.feature:10\n"
  )
end

# Heredoc strings are static values for block style too
it do
  allow(provider).to receive(:read_crontab).and_return(<<~CRONTAB)
                                            ^^^^^^^^^^ RSpec/ReturnFromStub: Use block for static values.
    0 2 * * * /some/other/command
  CRONTAB
end
