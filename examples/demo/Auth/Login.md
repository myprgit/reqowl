# Login flow

Log in first. `@name login` lets later blocks use this response.

```http
# @name login
POST {{api}}/login
Content-Type: application/json

{"user": "{{user}}", "password": "{{password}}"}
```

The token from the response above goes into the next request:

```http
# @auth bearer {{login.response.body.$.token}}
GET {{api}}/orders
  ?limit=2
```

The same data over GraphQL; variables go after a blank line:

```http
# @auth bearer {{login.response.body.$.token}}
GRAPHQL {{api}}/graphql

query Me { me { name orders } }
```
