# Matching Engine Example Application

This sample application demonstrates a matching engine, showcasing the DeFi capabilities
on the Linera protocol.

The matching engine trades between two tokens `token_0` & `token_1`. We can refer to
the `fungible` application example on how to create two token applications.

An order can be of two types:

- Bid: For buying token 1 and paying in token 0, these are ordered from the highest
  bid (most preferable) to the lowest price.
- Ask: For selling token 1, to be paid in token 0, these are ordered from the lowest
  (most preferable) to the highest price.

An `OrderId` is used to uniquely identify an order and enables the following functionality:

- Modify: Allows to modify the order.
- Cancel: Cancelling an order.

When inserting an order it goes through the following steps:

- Transfer of tokens from the `fungible` application to the `matching engine` application through a cross-application
  call so that it can be paid to the counterparty.

- The engine selects the matching price levels for the inserted order. It then proceeds
  to clear these levels, executing trades and ensuring that at the end of the process,
  the best bid has a higher price than the best ask. This involves adjusting the orders in the market
  and potentially creating a series of transfer orders for the required tokens. If, after
  the level clearing, the order is completely filled, it is not inserted. Otherwise,
  it becomes a liquidity order in the matching engine, providing depth to the market
  and potentially being matched with future orders.

When an order is created from a remote chain, it transfers the tokens of the same owner
from the remote chain to the chain of the matching engine, and a `ExecuteOrder` message is sent with the order details.

## Usage

### Setting Up

Before getting started, make sure that the binary tools `linera*` corresponding to
your version of `linera-sdk` are in your PATH. For scripting purposes, we also assume
that the BASH function `linera_spawn` is defined.

From the root of Linera repository, this can be achieved as follows:

```bash
export PATH="$PWD/target/debug:$PATH"
source /dev/stdin <<<"$(linera net helper 2>/dev/null)"
```

Next, start the local Linera network and run a faucet:

```bash
FAUCET_PORT=8079
FAUCET_URL=http://localhost:$FAUCET_PORT
linera_spawn linera net up --with-faucet --faucet-port $FAUCET_PORT

# If you're using a testnet, run this instead:
#   LINERA_TMP_DIR=$(mktemp -d)
#   FAUCET_URL=https://faucet.testnet-XXX.linera.net  # for some value XXX
```

Create the user wallet and add chains to it:

```bash
export LINERA_WALLET="$LINERA_TMP_DIR/wallet.json"
export LINERA_KEYSTORE="$LINERA_TMP_DIR/keystore.json"
export LINERA_STORAGE="rocksdb:$LINERA_TMP_DIR/client.db"

linera wallet init --faucet $FAUCET_URL

INFO_1=($(linera wallet request-chain --faucet $FAUCET_URL))
INFO_2=($(linera wallet request-chain --faucet $FAUCET_URL))
INFO_3=($(linera wallet request-chain --faucet $FAUCET_URL))
CHAIN_1="${INFO_1[0]}"
CHAIN_2="${INFO_2[0]}"
CHAIN_3="${INFO_3[0]}"
OWNER_1="${INFO_1[1]}"
OWNER_2="${INFO_2[1]}"
OWNER_3="${INFO_3[1]}"
```

Publish and create two `fungible` applications whose IDs will be used as a
parameter for the `matching-engine` application:

```bash
FUN1_APP_ID=$(linera --wait-for-outgoing-messages \
  project publish-and-create examples/fungible \
    --json-argument "{ \"accounts\": {
        \"$OWNER_1\": \"100.\",
        \"$OWNER_2\": \"150.\"
    } }" \
    --json-parameters "{ \"ticker_symbol\": \"FUN1\" }" \
)

FUN2_APP_ID=$(linera --wait-for-outgoing-messages \
  project publish-and-create examples/fungible \
    --json-argument "{ \"accounts\": {
        \"$OWNER_1\": \"100.\",
        \"$OWNER_2\": \"150.\"
    } }" \
    --json-parameters "{ \"ticker_symbol\": \"FUN2\" }" \
)
```

The flag `--wait-for-outgoing-messages` waits until a quorum of validators has confirmed
that all sent cross-chain messages have been delivered.

Now, we publish and deploy the Matching Engine application:

```bash
MATCHING_ENGINE=$(linera --wait-for-outgoing-messages \
    project publish-and-create examples/matching-engine \
    --json-parameters "{\"tokens\":["\"$FUN1_APP_ID\"","\"$FUN2_APP_ID\""]}" \
    --required-application-ids $FUN1_APP_ID $FUN2_APP_ID)
```

### Using the Matching Engine Application

First, a node service for the current wallet has to be started:

```bash
PORT=8080
linera service --port $PORT &
```

#### Using GraphiQL

Type each of these in the GraphiQL interface and substitute the env variables with their actual values that we've defined above.

Navigate to the URL you get by running `echo "http://localhost:8080/chains/$CHAIN_1/applications/$MATCHING_ENGINE"`:

To create a `Bid` order as owner 1, offering to buy 1 FUN1 for 5 FUN2:

```gql,uri=http://localhost:8080/chains/$CHAIN_1/applications/$MATCHING_ENGINE
mutation {
  executeOrder(
    order: {
        Insert : {
        owner: "$OWNER_1",
        amount: "1",
        nature: Bid,
        price: {
            price:5
        }
      }
    }
  )
}
```

To query about the bid price:

```gql,uri=http://localhost:8080/chains/$CHAIN_1/applications/$MATCHING_ENGINE
query {
  bids {
    keys {
      price
    }
  }
}
```

#### Atomic Swaps

In general, if you send tokens to a chain owned by someone else, you rely on them
for asset availability: If they don't handle your messages, you don't have access to
your tokens.

Fortunately, Linera provides a solution based on temporary chains:
If the number of parties who want to swap tokens is limited, we can make them all chain
owners, allow only Matching Engine operations on the chain, and allow only the Matching
Engine to close the chain.

```bash
kill %% && sleep 1    # Kill the service so we can use CLI commands for chain 1.

linera --wait-for-outgoing-messages change-ownership \
    --owners $OWNER_1 $OWNER_2

linera --wait-for-outgoing-messages change-application-permissions \
    --execute-operations $MATCHING_ENGINE \
    --close-chain $MATCHING_ENGINE

linera service --port $PORT &
```

First, owner 2 should claim their tokens. Navigate to the URL you get by running `echo "http://localhost:8080/chains/$CHAIN_2/applications/$FUN1_APP_ID"`:

```gql,uri=http://localhost:8080/chains/$CHAIN_2/applications/$FUN1_APP_ID
mutation {
  claim(
    sourceAccount: {
      chainId: "$CHAIN_1",
      owner: "$OWNER_2",
    }
    amount: "100.",
    targetAccount: {
      chainId: "$CHAIN_2",
      owner: "$OWNER_3"
    }
  )
}
```

And to the URL you get by running `echo "http://localhost:8080/chains/$CHAIN_2/applications/$FUN2_APP_ID"`:

```gql,uri=http://localhost:8080/chains/$CHAIN_2/applications/$FUN2_APP_ID
mutation {
  claim(
    sourceAccount: {
      chainId: "$CHAIN_1",
      owner: "$OWNER_2",
    }
    amount: "150.",
    targetAccount: {
      chainId: "$CHAIN_2",
      owner: "$OWNER_2"
    }
  )
}
```

Owner 2 offers to buy 2 FUN1 for 10 FUN2. This gets partially filled, and they buy 1 FUN1
for 5 FUN2 from owner 1. This leaves 5 FUN2 of owner 2 on chain 1. On the URL you get by running `echo "http://localhost:8080/chains/$CHAIN_2/applications/$MATCHING_ENGINE"`:

```gql,uri=http://localhost:8080/chains/$CHAIN_2/applications/$MATCHING_ENGINE
mutation {
  executeOrder(
    order: {
      Insert : {
        owner: "$OWNER_2",
        amount: "2",
        nature: Ask,
        price: {
            price:5
        }
      }
    }
  )
}
```

The only way to close the chain is via the application now. On the URL you get by running `echo "http://localhost:8080/chains/$CHAIN_1/applications/$MATCHING_ENGINE"`:

```gql,uri=http://localhost:8080/chains/$CHAIN_1/applications/$MATCHING_ENGINE
mutation { closeChain }
```

Owner 2 should now get back their tokens, and have 145 FUN2 left. On the URL you get by running `echo "http://localhost:8080/chains/$CHAIN_2/applications/$FUN2_APP_ID"`

```gql,uri=http://localhost:8080/chains/$CHAIN_2/applications/$FUN2_APP_ID
query {
  accounts {
    entry(
      key: "$OWNER_2"
    ) {
      value
    }
  }
}
```
