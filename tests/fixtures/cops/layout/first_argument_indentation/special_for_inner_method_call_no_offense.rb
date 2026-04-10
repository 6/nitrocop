variations << normalized_uri_string.sub(
                /#{Regexp.escape(normalized_uri.request_uri)}$/,
                ":#{normalized_uri.port}#{normalized_uri.request_uri}"
              )
