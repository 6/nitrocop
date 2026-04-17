# nitrocop-config: EnforcedStyle: table, table, ignore_implicit
let(:loaded_records) do
  {
  "concern_resources#show" =>
  ^^^^^^^^^^^^^^^^^^^^^^^^^^^ Layout/HashAlignment: Align the keys and values of a hash literal if they span more than one line.
    [{
      "verb"=>:GET,
      ^^^^^^^^^^^^ Layout/HashAlignment: Align the keys and values of a hash literal if they span more than one line.
      "path"=>"/api/concerns/5",
      ^^^^^^^^^^^^^^^^^^^^^^^^^ Layout/HashAlignment: Align the keys and values of a hash literal if they span more than one line.
      "versions"=>["development"],
      ^^^^^^^^^^^^^^^^^^^^^^^^^^^ Layout/HashAlignment: Align the keys and values of a hash literal if they span more than one line.
      "query"=>"session=secret_hash",
      ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Layout/HashAlignment: Align the keys and values of a hash literal if they span more than one line.
      "request_data"=>nil,
      ^^^^^^^^^^^^^^^^^^^ Layout/HashAlignment: Align the keys and values of a hash literal if they span more than one line.
      "response_data"=>"OK {\"session\"=>\"secret_hash\", \"id\"=>\"5\", \"controller\"=>\"concerns\", \"action\"=>\"show\"}",
      ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Layout/HashAlignment: Align the keys and values of a hash literal if they span more than one line.
      "code"=>"200"
      ^^^^^^^^^^^^^ Layout/HashAlignment: Align the keys and values of a hash literal if they span more than one line.
    }]
  }
end
