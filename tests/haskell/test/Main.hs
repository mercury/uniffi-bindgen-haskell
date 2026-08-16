{-# LANGUAGE OverloadedRecordDot #-}

module Main (main) where

import Control.Concurrent (forkIO, newEmptyMVar, putMVar, takeMVar)
import Control.Exception (try)
import Control.Monad (forM, forM_, unless, void)
import qualified Data.ByteString as ByteString
import Data.Int (Int16, Int32, Int64, Int8)
import qualified Data.Map.Strict as Map
import qualified Data.Set as Set
import qualified Data.Text as Text
import Data.Word (Word16, Word32, Word64, Word8)
import System.Timeout (timeout)
import UniFFI.Runtime
  ( Duration (..)
  , Timestamp (..)
  , UniFFIException
  )
import qualified UniFFI.UniffiHaskellExternalFixture as External
import UniFFI.UniffiHaskellFixture

main :: IO ()
main = do
  initialize
  ping
  runGroup "primitives" testPrimitives
  runGroup "buffers" testBuffers
  runGroup "compound types" testCompoundTypes
  runGroup "remaining value types" testRemainingValueTypes
  runGroup "defaults and renames" testDefaultsAndRenames
  runGroup "errors" testErrors
  runGroup "async" testAsync
  runGroup "callbacks" testCallbacks
  runGroup "objects" testObjects
  runGroup "panic translation" testPanic

runGroup :: String -> IO () -> IO ()
runGroup name action = do
  putStrLn ("Running " ++ name)
  action

assertEqual :: (Eq a, Show a) => String -> a -> a -> IO ()
assertEqual label expected actual =
  unless (expected == actual) $
    fail (label ++ ": expected " ++ show expected ++ ", got " ++ show actual)

testPrimitives :: IO ()
testPrimitives = do
  addResult <- add 20 22
  assertEqual "add" 42 addResult
  roundtripU8 minBound >>= assertEqual "u8 min" (minBound :: Word8)
  roundtripU8 maxBound >>= assertEqual "u8 max" (maxBound :: Word8)
  roundtripI8 minBound >>= assertEqual "i8 min" (minBound :: Int8)
  roundtripI8 maxBound >>= assertEqual "i8 max" (maxBound :: Int8)
  roundtripU16 maxBound >>= assertEqual "u16 max" (maxBound :: Word16)
  roundtripI16 minBound >>= assertEqual "i16 min" (minBound :: Int16)
  roundtripU32 maxBound >>= assertEqual "u32 max" (maxBound :: Word32)
  roundtripI32 minBound >>= assertEqual "i32 min" (minBound :: Int32)
  roundtripU64 maxBound >>= assertEqual "u64 max" (maxBound :: Word64)
  roundtripI64 minBound >>= assertEqual "i64 min" (minBound :: Int64)
  roundtripF32 (-123.5) >>= assertEqual "f32" (-123.5)
  roundtripF64 (1 / 3) >>= assertEqual "f64" (1 / 3)
  roundtripBool True >>= assertEqual "bool true" True
  roundtripBool False >>= assertEqual "bool false" False
  mixedSum <- sumMixedPrimitives 1 2 3 4 5 6 7 8 9 10 False
  assertEqual "mixed primitive sum" 55 mixedSum
  negated <- sumMixedPrimitives 1 2 3 4 5 6 7 8 9 10 True
  assertEqual "mixed primitive negation" (-55) negated

testBuffers :: IO ()
testBuffers = do
  let text = Text.pack "Mercury 🚀 e\x0301\NUL"
      bytes = ByteString.pack [0, 1, 2, 127, 128, 254, 255]
  roundtripString text >>= assertEqual "Unicode string" text
  roundtripString Text.empty >>= assertEqual "empty string" Text.empty
  roundtripBytes bytes >>= assertEqual "bytes" bytes
  roundtripBytes ByteString.empty >>= assertEqual "empty bytes" ByteString.empty

testCompoundTypes :: IO ()
testCompoundTypes = do
  let person =
        Person
          { name = Text.pack "Ada Lovelace"
          , age = 36
          , nickname = Just (Text.pack "Enchantress of Numbers")
          , scores = [minBound, -1, 0, 1, maxBound]
          , avatar = ByteString.pack [0, 128, 255]
          }
      otherPerson =
        Person
          { name = Text.pack "Grace Hopper"
          , age = 85
          , nickname = Nothing
          , scores = []
          , avatar = ByteString.empty
          }
  assertEqual "named field name" (Text.pack "Ada Lovelace") person.name
  assertEqual "named field age" (36 :: Word8) person.age
  assertEqual
    "named field nickname"
    (Just (Text.pack "Enchantress of Numbers"))
    person.nickname
  assertEqual "named field scores" [minBound, -1, 0, 1, maxBound] person.scores
  assertEqual "named field avatar" (ByteString.pack [0, 128, 255]) person.avatar
  let updatedPerson = person {age = 37, nickname = Nothing}
  assertEqual "named record update age" (37 :: Word8) updatedPerson.age
  assertEqual "named record update nickname" Nothing updatedPerson.nickname
  roundtripPerson person >>= assertEqual "record" person
  roundtripPerson updatedPerson >>= assertEqual "updated record" updatedPerson
  roundtripOptionalPerson (Just person) >>= assertEqual "optional record" (Just person)
  roundtripOptionalPerson Nothing >>= assertEqual "empty optional record" Nothing
  roundtripPeople [person, otherPerson] >>= assertEqual "record sequence" [person, otherPerson]
  roundtripStrings [Text.empty, Text.pack "α", Text.pack "🚀"]
    >>= assertEqual "string sequence" [Text.empty, Text.pack "α", Text.pack "🚀"]
  roundtripStatus StatusIdle >>= assertEqual "unit enum" StatusIdle
  let message = StatusMessage (Text.pack "working")
  roundtripStatus message >>= assertEqual "named enum payload" message
  let detailed = StatusDetailed maxBound (Text.pack "details")
  roundtripStatus detailed >>= assertEqual "unnamed enum payload" detailed

testRemainingValueTypes :: IO ()
testRemainingValueTypes = do
  let stringMap = Map.fromList [(Text.pack "negative", -7), (Text.pack "positive", 42)]
      stringSet = Set.fromList [Text.empty, Text.pack "alpha", Text.pack "🚀"]
      afterEpoch = Timestamp 123456789 987654321
      beforeEpoch = Timestamp (-123456789) 123456789
      duration = Duration maxBound 999999999
      tree =
        TreeNode
          (TreeLeaf minBound)
          (TreeNode (TreeLeaf 0) (TreeLeaf maxBound))
      bytes = ByteString.pack [1, 2, 3, 250]
  putStrLn "  map"
  roundtripHashMap stringMap >>= assertEqual "map" stringMap
  putStrLn "  set"
  roundtripHashSet stringSet >>= assertEqual "set" stringSet
  putStrLn "  timestamp after epoch"
  roundtripSystemTime afterEpoch >>= assertEqual "timestamp after epoch" afterEpoch
  putStrLn "  timestamp before epoch"
  roundtripSystemTime beforeEpoch >>= assertEqual "timestamp before epoch" beforeEpoch
  putStrLn "  duration"
  roundtripDuration duration >>= assertEqual "duration" duration
  putStrLn "  custom integer"
  roundtripUserId (UserId maxBound) >>= assertEqual "custom integer" (UserId maxBound)
  putStrLn "  custom string"
  roundtripLabel (Label (Text.pack "custom 🚀"))
    >>= assertEqual "custom string" (Label (Text.pack "custom 🚀"))
  putStrLn "  recursive enum"
  roundtripTree tree >>= assertEqual "recursive enum" tree
  putStrLn "  borrowed bytes"
  sumBytes bytes >>= assertEqual "borrowed bytes" 256
  external <- External.makeExternalRecord (Text.pack "external") 42
  let expectedExternal = External.ExternalRecord (Text.pack "external") 42
  assertEqual "external namespace function" expectedExternal external
  assertEqual "external record dot name" (Text.pack "external") external.name
  assertEqual "external record dot value" 42 external.value
  roundtripExternalRecord external >>= assertEqual "external type roundtrip" expectedExternal

testDefaultsAndRenames :: IO ()
testDefaultsAndRenames = do
  let person = Person (Text.pack "Borrowed") 1 Nothing [] ByteString.empty
      renamed = RenamedRecord (Text.pack "renamed")
      expectedDefaults =
        DefaultsRecord
          { boolean = False
          , integer = 42
          , optionalString = Nothing
          , strings = []
          , map_ = Map.empty
          , set = Set.empty
          }
  personName person >>= assertEqual "borrowed record" (Text.pack "Borrowed")
  renamedFunction renamed >>= assertEqual "renamed API" renamed
  assertEqual "record defaults" expectedDefaults defaultDefaultsRecord
  roundtripDefaultsRecord defaultDefaultsRecord
    >>= assertEqual "default record roundtrip" expectedDefaults
  doubleWithDefault 5 >>= assertEqual "explicit default argument" 10
  doubleWithDefaultUsingDefaults >>= assertEqual "generated default wrapper" 42

testErrors :: IO ()
testErrors = do
  divide 84 2 >>= assertEqual "error success" (Right 42)
  divide 1 0 >>= assertEqual "unit error" (Left TestErrorDivisionByZero)
  divide minBound (-1)
    >>= assertEqual
      "named error payload"
      (Left (TestErrorInvalidDivision (Text.pack "integer overflow")))
  divide 9 (-3)
    >>= assertEqual "unnamed error payload" (Left (TestErrorNegativeDivisor 3))

testAsync :: IO ()
testAsync = do
  asyncPing
  asyncAdd 20 22 >>= assertEqual "async scalar" 42
  let person = Person (Text.pack "Async") 7 (Just (Text.pack "future")) [1, 2] ByteString.empty
  asyncRoundtripPerson person >>= assertEqual "async buffer" person
  asyncDivide 84 2 >>= assertEqual "async success" (Right 42)
  asyncDivide 1 0 >>= assertEqual "async typed error" (Left TestErrorDivisionByZero)
  cancelled <- timeout 100000 asyncNever
  assertEqual "async cancellation" Nothing cancelled
  completions <- forM [1 .. 10 :: Word32] $ \value -> do
    completion <- newEmptyMVar
    void $
      forkIO $ do
        result <- asyncAdd value value
        putMVar completion (value, result)
    pure completion
  results <- mapM takeMVar completions
  forM_ results $ \(value, result) ->
    assertEqual "concurrent async calls" (value + value) result

testCallbacks :: IO ()
testCallbacks = do
  let callback =
        TestCallback
          { testCallbackTransform = \value -> pure (value * 2)
          , testCallbackDescribe = \value -> pure (value <> Text.pack " described")
          , testCallbackFallible = \value ->
              if value == 0
                then pure (Left TestErrorDivisionByZero)
                else pure (Right (value + 1))
          }
  invokeCallbackTransform callback 21 >>= assertEqual "callback scalar" 42
  invokeCallbackDescribe callback (Text.pack "value")
    >>= assertEqual "callback buffer" (Text.pack "value described")
  invokeCallbackFallible callback 9 >>= assertEqual "callback success" (Right 10)
  invokeCallbackFallible callback 0
    >>= assertEqual "callback expected error" (Left TestErrorDivisionByZero)
  invokeCallbackConcurrently callback [1 .. 20]
    >>= assertEqual "callbacks from Rust threads" (map (* 2) [1 .. 20])

  let throwingCallback =
        callback
          { testCallbackTransform = \_ -> ioError (userError "callback panic")
          , testCallbackFallible = \_ -> ioError (userError "fallible callback panic")
          }
  unexpected <- try (invokeCallbackTransform throwingCallback 1) :: IO (Either UniFFIException Int32)
  case unexpected of
    Left _ -> pure ()
    Right value -> fail ("unexpected callback returned " ++ show value)
  converted <- invokeCallbackFallible throwingCallback 1
  case converted of
    Left (TestErrorUnexpectedCallback message) ->
      unless (Text.pack "fallible callback panic" `Text.isInfixOf` message) $
        fail ("unexpected callback message: " ++ show message)
    other -> fail ("unexpected callback conversion result: " ++ show other)

testObjects :: IO ()
testObjects = do
  counter <- newCounter 10
  counterGet counter >>= assertEqual "object initial value" 10
  counterAdd counter 5 >>= assertEqual "object method result" 15
  counterSumBytes counter (ByteString.pack [1, 2, 3])
    >>= assertEqual "object borrowed bytes" 6
  let person = Person (Text.pack "Object") 2 Nothing [1] ByteString.empty
  counterRoundtripPerson counter person >>= assertEqual "object buffer method" person
  counterFallibleGet counter False >>= assertEqual "object fallible success" (Right 15)
  counterFallibleGet counter True
    >>= assertEqual "object fallible error" (Left TestErrorDivisionByZero)
  counterAsyncGet counter >>= assertEqual "object async method" 15
  clonedCounter <- roundtripCounter counter
  counterAdd clonedCounter 1 >>= assertEqual "object argument and return" 16
  counterGet counter >>= assertEqual "shared object identity" 16
  closeCounter clonedCounter
  completions <- forM [1 .. 20 :: Int] $ \_ -> newEmptyMVar
  forM_ completions $ \completion ->
    void $
      forkIO $ do
        void (counterAdd counter 1)
        putMVar completion ()
  mapM_ takeMVar completions
  counterGet counter >>= assertEqual "concurrent object methods" 36
  closeCounter counter
  closeCounter counter
  closedResult <- try (counterGet counter) :: IO (Either UniFFIException Int64)
  case closedResult of
    Left _ -> pure ()
    Right value -> fail ("closed object returned " ++ show value)

testPanic :: IO ()
testPanic = do
  result <- try panicNow :: IO (Either UniFFIException ())
  case result of
    Left _ -> pure ()
    Right () -> fail "Rust panic unexpectedly succeeded"
