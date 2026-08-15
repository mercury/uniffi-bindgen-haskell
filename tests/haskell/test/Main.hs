module Main (main) where

import Control.Concurrent (forkIO, newEmptyMVar, putMVar, takeMVar)
import Control.Exception (try)
import Control.Monad (forM, forM_, unless, void)
import qualified Data.ByteString as ByteString
import Data.Int (Int16, Int32, Int64, Int8)
import qualified Data.Text as Text
import Data.Word (Word16, Word32, Word64, Word8)
import UniFFI.Runtime (UniFFIException)
import UniFFI.UniffiHaskellFixture

main :: IO ()
main = do
  initialize
  ping
  testPrimitives
  testBuffers
  testCompoundTypes
  testErrors
  testObjects
  testPanic

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
  roundtripPerson person >>= assertEqual "record" person
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

testObjects :: IO ()
testObjects = do
  counter <- newCounter 10
  counterGet counter >>= assertEqual "object initial value" 10
  counterAdd counter 5 >>= assertEqual "object method result" 15
  completions <- forM [1 .. 20 :: Int] $ \_ -> newEmptyMVar
  forM_ completions $ \completion ->
    void $
      forkIO $ do
        void (counterAdd counter 1)
        putMVar completion ()
  mapM_ takeMVar completions
  counterGet counter >>= assertEqual "concurrent object methods" 35
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
