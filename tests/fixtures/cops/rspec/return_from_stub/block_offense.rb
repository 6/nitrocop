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
