response.parsed_body
JSON.parse(data)
JSON.parse(file.read)
response.body
JSON.parse(request.body)
# RuboCop's node pattern requires `response` to have a nil receiver:
# (send (const {nil? cbase} :JSON) :parse (send (send nil? :response) :body))
JSON.parse(error.response.body)
Nokogiri::XML(response.body)
# Extra arguments — RuboCop only matches exactly 1 argument
JSON.parse(response.body, symbolize_names: true)
JSON.parse(response.body, symbolize_names: false)
Nokogiri::HTML.parse(response.body, nil, "UTF-8")
Nokogiri::HTML5.parse(response.body, max_errors: 10)
# Nokogiri patterns require target_rails_version >= 7.1 (tested at 5.0 here)
Nokogiri::HTML.parse(response.body)
Nokogiri::HTML5.parse(response.body)
