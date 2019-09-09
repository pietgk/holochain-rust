const path = require('path')
const tape = require('tape')

const { Orchestrator, tapeExecutor, backwardCompatibilityMiddleware, compose } = require('@holochain/try-o-rama')
const spawnConductor = require('./spawn_conductors')

// This constant serves as a check that we haven't accidentally disabled scenario tests.
// Try to keep this number as close as possible to the actual number of scenario tests.
// (But never over)
const MIN_EXPECTED_SCENARIOS = 49

process.on('unhandledRejection', error => {
  // Will print "unhandledRejection err is not defined"
  console.error('got unhandledRejection:', error);
});

const dnaPath = path.join(__dirname, "../dist/app_spec.dna.json")
const dna = Orchestrator.dna(dnaPath, 'app-spec')
const dna2 = Orchestrator.dna(dnaPath, 'app-spec', {uuid: 'altered-dna'})

// map e.g. `alice.app.call` ~> `conductor.alice.call`
const inMemoryMiddleware = f => (api, {conductor}) => {
  const conductorMap = {}
  Object.keys(conductor).forEach(name => {
    const inst = conductor[name]
    if (name !== '_conductor') {
      conductorMap[name] = {
        app: inst
      }
    }
  })
  return f(api, conductorMap)
}

const commonConductorConfig = {
  instances: {
    app: dna,
  },
}

const orchestratorSimple = new Orchestrator({
  conductors: {
    conductor: {
      instances: {
        alice: dna,
        bob: dna,
        carol: dna,
      }
    }
  },
  debugLog: false,
  executor: tapeExecutor(require('tape')),
  middleware: compose(
    backwardCompatibilityMiddleware,
    inMemoryMiddleware,
  ),
})

const orchestratorMultiDna = new Orchestrator({
  conductors: {
    conductor: {
      instances: {
        app1: dna,
        app2: dna2,
      },
      bridges: [
        Orchestrator.bridge('test-bridge', 'app1', 'app2')
      ],
    }
  },
  debugLog: false,
  executor: tapeExecutor(require('tape')),
  middleware: backwardCompatibilityMiddleware,
  callbacksPort: 8888,
})

const orchestratorValidateAgent = new Orchestrator({
  conductors: {
    valid_agent: { instances: { app: dna } },
    reject_agent: { instances: { app: dna } },
  },
  debugLog: false,
  executor: tapeExecutor(require('tape')),
  middleware: backwardCompatibilityMiddleware,
})

const registerAllScenarios = () => {
  // NB: all scenarios must be registered before any orchestrator is run. Tape will fail to register its
  // test cases if there is any Promise awaiting in between test declarations.
  let numRegistered = 0

  const registerer = orchestrator => {
    const f = (...info) => {
      numRegistered += 1
      return orchestrator.registerScenario(...info)
    }
    f.only = (...info) => {
      numRegistered += 1
      return orchestrator.registerScenario.only(...info)
    }
    return f
  }

  require('./regressions')(registerer(orchestratorSimple))
  require('./test')(registerer(orchestratorSimple))
  // require('./multi-dna')(registerer(orchestratorMultiDna))
  require('./validate-agent-test')(registerer(orchestratorValidateAgent))

  return numRegistered
}


const runSimpleTests = async () => {
  const conductor = await spawnConductor('conductor', 3000, false)
  await orchestratorSimple.registerConductor({name: 'conductor', url: 'http://0.0.0.0:3000'})

  const delay = ms => new Promise(resolve => setTimeout(resolve, ms))
  console.log("Waiting for conductors to settle...")
  await delay(5000)
  console.log("Ok, starting tests!")

  await orchestratorSimple.run()
  conductor.kill()
}

const runMultiDnaTests = async () => {
  // Multi instance tests where n3h is the network connecting them currently fails with the 2nd instance
  // waiting for and not receiving the agent entry of the first one.
  // I believe this is due to n3h not sending a peer connected message for a local instance
  // and core has not implented the authoring list yet...
  const conductor = await spawnConductor('conductor', 6000, true)
  await orchestratorMultiDna.registerConductor({name: 'conductor', url: 'http://0.0.0.0:6000'})
  await orchestratorMultiDna.run()
  conductor.kill()
}

const runValidationTests = async () => {
  const valid_agent = await spawnConductor('valid_agent', 3000, true)
  await orchestratorValidateAgent.registerConductor({name: 'valid_agent', url: 'http://0.0.0.0:3000'})
  const reject_agent = await spawnConductor('reject_agent', 4000, true)
  await orchestratorValidateAgent.registerConductor({name: 'reject_agent', url: 'http://0.0.0.0:4000'})

  const delay = ms => new Promise(resolve => setTimeout(resolve, ms))
  console.log("Waiting for conductors to settle...")
  await delay(5000)
  console.log("Ok, starting tests!")

  await orchestratorValidateAgent.run()
  valid_agent.kill()
  reject_agent.kill()
}

const run = async () => {
  const num = registerAllScenarios()

  // Check to see that we haven't accidentally disabled a bunch of scenarios
  if (num < MIN_EXPECTED_SCENARIOS) {
    console.error(`Expected at least ${MIN_EXPECTED_SCENARIOS}, but only ${num} were registered!`)
    process.exit(1)
  } else {
    console.log(`Registered ${num} scenarios (at least ${MIN_EXPECTED_SCENARIOS} were expected)`)
  }

  await runSimpleTests()
  // await runMultiDnaTests()
  // await runValidationTests()
  process.exit()
}

run()