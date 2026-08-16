module Main (main) where

import Control.Concurrent
  ( forkIO
  , modifyMVar_
  , newEmptyMVar
  , newMVar
  , putMVar
  , readMVar
  , takeMVar
  , tryPutMVar
  , tryReadMVar
  )
import Control.Exception (Exception, SomeException, evaluate, fromException, throwIO, try)
import Control.Monad (replicateM, replicateM_, unless, void)
import Data.ByteString (ByteString)
import qualified Data.ByteString as ByteString
import Data.Int (Int8, Int16, Int32, Int64)
import Data.IORef (IORef, newIORef)
import Data.Map.Strict (Map)
import Data.Maybe (isJust)
import qualified Data.Map.Strict as Map
import Data.Set (Set)
import qualified Data.Set as Set
import qualified Data.Text as Text
import Data.Word (Word8, Word16, Word32, Word64)
import Foreign.Marshal.Alloc (allocaBytes)
import Foreign.Marshal.Utils (fillBytes)
import Foreign.Ptr (Ptr, castPtr, intPtrToPtr, nullPtr)
import Foreign.Storable (Storable (..), peekByteOff)
import Prelude hiding (readList)
import UniFFI.Runtime

newtype InitializationFailure = InitializationFailure (IORef ())

instance Show InitializationFailure where
  show _ = "InitializationFailure"

instance Exception InitializationFailure

main :: IO ()
main = do
  testInitializationSuccess
  testInitializationFailure
  testInitializationReentrancy
  testStorableLayouts
  testSuccessStatus
  testUtf8
  testBytesSerialization
  testNumericCodecs
  testBoolCodec
  testTextCodec
  testBytesCodec
  testMaybeAndListCodecs
  testMapAndSetCodecs
  testTimeCodecs
  testDecoderFailures

assertEqual :: (Eq a, Show a) => String -> a -> a -> IO ()
assertEqual label expected actual =
  unless (expected == actual) $
    fail (label ++ ": expected " ++ show expected ++ ", got " ++ show actual)

assertSucceeded :: String -> Either SomeException () -> IO ()
assertSucceeded label result =
  case result of
    Left exception -> fail (label ++ ": unexpected exception: " ++ show exception)
    Right () -> pure ()

assertInitializationFailure :: String -> IORef () -> Either SomeException () -> IO ()
assertInitializationFailure label expectedToken result =
  case result of
    Left exception ->
      case fromException exception of
        Just (InitializationFailure actualToken) ->
          unless (actualToken == expectedToken) $
            fail (label ++ ": received a different InitializationFailure")
        Nothing -> fail (label ++ ": unexpected exception: " ++ show exception)
    Right () -> fail (label ++ ": initialization unexpectedly succeeded")

assertCodec :: (Eq a, Show a) => String -> ByteString -> (a -> Encoder) -> Decoder a -> a -> IO ()
assertCodec label expected writeValue readValue value = do
  assertEqual (label ++ " encoding") expected (runEncoder (writeValue value))
  decoded <- runDecoder readValue expected
  assertEqual (label ++ " decoding") value decoded

assertDecoderFails :: String -> Decoder a -> ByteString -> IO ()
assertDecoderFails label decoder input = do
  result <- try (runDecoder decoder input >> pure ()) :: IO (Either UniFFIException ())
  case result of
    Left _ -> pure ()
    Right () -> fail (label ++ ": decoder unexpectedly succeeded")

assertEncoderFails :: String -> Encoder -> IO ()
assertEncoderFails label encoder = do
  result <- try (evaluate (ByteString.length (runEncoder encoder))) :: IO (Either UniFFIException Int)
  case result of
    Left _ -> pure ()
    Right _ -> fail (label ++ ": encoder unexpectedly succeeded")

alignUp :: Int -> Int -> Int
alignUp offset boundary = ((offset + boundary - 1) `div` boundary) * boundary

testInitializationSuccess :: IO ()
testInitializationSuccess = do
  initialization <- newInitialization
  executionCount <- newMVar (0 :: Int)
  ready <- newEmptyMVar
  start <- newEmptyMVar
  actionStarted <- newEmptyMVar
  finishAction <- newEmptyMVar
  let callerCount = 32
      action = do
        modifyMVar_ executionCount (pure . (+ 1))
        void (tryPutMVar actionStarted ())
        readMVar finishAction
      caller resultVariable = do
        putMVar ready ()
        readMVar start
        result <- try (runInitialization initialization action)
        putMVar resultVariable result
  resultVariables <- replicateM callerCount newEmptyMVar
  mapM_ (forkIO . caller) resultVariables
  replicateM_ callerCount (takeMVar ready)
  putMVar start ()
  takeMVar actionStarted
  blockedExecutionCount <- readMVar executionCount
  assertEqual "concurrent initialization execution count while blocked" 1 blockedExecutionCount
  completedBeforeRelease <- mapM (fmap isJust . tryReadMVar) resultVariables
  assertEqual
    "concurrent initialization callers wait"
    (replicate callerCount False)
    completedBeforeRelease
  putMVar finishAction ()
  results <- mapM takeMVar resultVariables
  mapM_ (assertSucceeded "concurrent initialization") results
  finalExecutionCount <- readMVar executionCount
  assertEqual "concurrent initialization execution count" 1 finalExecutionCount
  runInitialization initialization (modifyMVar_ executionCount (pure . (+ 1)))
  cachedExecutionCount <- readMVar executionCount
  assertEqual "successful initialization is cached" 1 cachedExecutionCount

testInitializationFailure :: IO ()
testInitializationFailure = do
  initialization <- newInitialization
  executionCount <- newMVar (0 :: Int)
  token <- newIORef ()
  firstResult <-
    try $
      runInitialization initialization $ do
        modifyMVar_ executionCount (pure . (+ 1))
        throwIO (InitializationFailure token)
  assertInitializationFailure "initial initialization failure" token firstResult
  secondResult <-
    try $
      runInitialization initialization $
        modifyMVar_ executionCount (pure . (+ 1))
  assertInitializationFailure "cached initialization failure" token secondResult
  thirdResult <-
    try $
      runInitialization initialization $
        modifyMVar_ executionCount (pure . (+ 1))
  assertInitializationFailure "repeated cached initialization failure" token thirdResult
  finalExecutionCount <- readMVar executionCount
  assertEqual "failed initialization execution count" 1 finalExecutionCount

testInitializationReentrancy :: IO ()
testInitializationReentrancy = do
  initialization <- newInitialization
  executionCount <- newMVar (0 :: Int)
  runInitialization initialization $ do
    modifyMVar_ executionCount (pure . (+ 1))
    runInitialization initialization (modifyMVar_ executionCount (pure . (+ 100)))
  runInitialization initialization (modifyMVar_ executionCount (pure . (+ 100)))
  finalExecutionCount <- readMVar executionCount
  assertEqual "reentrant initialization execution count" 1 finalExecutionCount

testStorableLayouts :: IO ()
testStorableLayouts = do
  testRustBufferLayout
  testForeignBytesLayout
  testRustCallStatusLayout

testRustBufferLayout :: IO ()
testRustBufferLayout = do
  let wordSize = sizeOf (undefined :: Word64)
      pointerSize = sizeOf (undefined :: Ptr Word8)
      pointerAlignment = alignment (undefined :: Ptr Word8)
      structAlignment = max (alignment (undefined :: Word64)) pointerAlignment
      lenOffset = alignUp wordSize (alignment (undefined :: Word64))
      dataOffset = alignUp (lenOffset + wordSize) pointerAlignment
      expectedSize = alignUp (dataOffset + pointerSize) structAlignment
      dataPointer = intPtrToPtr 0x1234
      buffer = RustBuffer 21 13 dataPointer
  assertEqual "RustBuffer alignment" structAlignment (alignment buffer)
  assertEqual "RustBuffer size" expectedSize (sizeOf buffer)
  allocaBytes expectedSize $ \ptr -> do
    fillBytes ptr 0 expectedSize
    poke (castPtr ptr) buffer
    capacity <- peekByteOff ptr 0
    len <- peekByteOff ptr lenOffset
    dataPtr <- peekByteOff ptr dataOffset
    assertEqual "RustBuffer capacity offset" (21 :: Word64) capacity
    assertEqual "RustBuffer len offset" (13 :: Word64) len
    assertEqual "RustBuffer data offset" dataPointer dataPtr
    roundTrip <- peek (castPtr ptr)
    assertEqual "RustBuffer round trip" buffer roundTrip

testForeignBytesLayout :: IO ()
testForeignBytesLayout = do
  let intSize = sizeOf (undefined :: Int32)
      pointerSize = sizeOf (undefined :: Ptr Word8)
      pointerAlignment = alignment (undefined :: Ptr Word8)
      structAlignment = max (alignment (undefined :: Int32)) pointerAlignment
      dataOffset = alignUp intSize pointerAlignment
      expectedSize = alignUp (dataOffset + pointerSize) structAlignment
      dataPointer = intPtrToPtr 0x5678
      bytes = ForeignBytes 9 dataPointer
  assertEqual "ForeignBytes alignment" structAlignment (alignment bytes)
  assertEqual "ForeignBytes size" expectedSize (sizeOf bytes)
  allocaBytes expectedSize $ \ptr -> do
    fillBytes ptr 0 expectedSize
    poke (castPtr ptr) bytes
    len <- peekByteOff ptr 0
    dataPtr <- peekByteOff ptr dataOffset
    assertEqual "ForeignBytes len offset" (9 :: Int32) len
    assertEqual "ForeignBytes data offset" dataPointer dataPtr
    roundTrip <- peek (castPtr ptr)
    assertEqual "ForeignBytes round trip" bytes roundTrip

testRustCallStatusLayout :: IO ()
testRustCallStatusLayout = do
  let codeSize = sizeOf (undefined :: Int8)
      bufferAlignment = alignment (undefined :: RustBuffer)
      structAlignment = max (alignment (undefined :: Int8)) bufferAlignment
      errorOffset = alignUp codeSize bufferAlignment
      expectedSize = alignUp (errorOffset + sizeOf (undefined :: RustBuffer)) structAlignment
      errorBuffer = RustBuffer 0 0 nullPtr
      status = RustCallStatus 2 errorBuffer
  assertEqual "RustCallStatus alignment" structAlignment (alignment status)
  assertEqual "RustCallStatus size" expectedSize (sizeOf status)
  allocaBytes expectedSize $ \ptr -> do
    fillBytes ptr 0 expectedSize
    poke (castPtr ptr) status
    code <- peekByteOff ptr 0
    errorValue <- peekByteOff ptr errorOffset
    assertEqual "RustCallStatus code offset" (2 :: Int8) code
    assertEqual "RustCallStatus error offset" errorBuffer errorValue
    roundTrip <- peek (castPtr ptr)
    assertEqual "RustCallStatus round trip" status roundTrip

testSuccessStatus :: IO ()
testSuccessStatus = do
  (result, status) <-
    withRustCallStatus $ \statusPtr -> do
      initial <- peek statusPtr
      assertEqual "initialized call status" successRustCallStatus initial
      pure (42 :: Int)
  assertEqual "call result" 42 result
  assertEqual "peeked call status" successRustCallStatus status

testUtf8 :: IO ()
testUtf8 = do
  let value = Text.pack "Mercury \x1F680 e\x0301\NUL"
      encoded = encodeUtf8 value
  assertEqual "UTF-8 round trip" (Right value) (decodeUtf8 encoded)
  assertEqual "UTF-8 is strict" True (ByteString.length encoded > Text.length value)
  case decodeUtf8 (ByteString.pack [0xC3, 0x28]) of
    Left _ -> pure ()
    Right decoded -> fail ("invalid UTF-8 decoded as " ++ show decoded)

testBytesSerialization :: IO ()
testBytesSerialization = do
  let value = ByteString.pack [0, 1, 127, 128, 255]
      expected = ByteString.pack [0, 0, 0, 5, 0, 1, 127, 128, 255]
  serialized <- serializeBytes value
  assertEqual "bytes serialization" expected serialized
  deserialized <- deserializeBytes serialized
  assertEqual "bytes deserialization" value deserialized
  emptySerialized <- serializeBytes ByteString.empty
  assertEqual "empty bytes serialization" (ByteString.pack [0, 0, 0, 0]) emptySerialized
  emptyDeserialized <- deserializeBytes emptySerialized
  assertEqual "empty bytes deserialization" ByteString.empty emptyDeserialized

testNumericCodecs :: IO ()
testNumericCodecs = do
  assertCodec "Word8" (ByteString.pack [0xAB]) writeWord8 readWord8 (0xAB :: Word8)
  assertCodec "Word16" (ByteString.pack [0x12, 0x34]) writeWord16 readWord16 (0x1234 :: Word16)
  assertCodec
    "Word32"
    (ByteString.pack [0x89, 0xAB, 0xCD, 0xEF])
    writeWord32
    readWord32
    (0x89ABCDEF :: Word32)
  assertCodec
    "Word64"
    (ByteString.pack [0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF])
    writeWord64
    readWord64
    (0x0123456789ABCDEF :: Word64)
  assertCodec "Int8" (ByteString.pack [0xFE]) writeInt8 readInt8 (-2 :: Int8)
  assertCodec "Int16" (ByteString.pack [0xED, 0xCC]) writeInt16 readInt16 (-0x1234 :: Int16)
  assertCodec
    "Int32"
    (ByteString.pack [0xFE, 0xDC, 0xBA, 0x99])
    writeInt32
    readInt32
    (-0x01234567 :: Int32)
  assertCodec
    "Int64"
    (ByteString.pack [0xFE, 0xDC, 0xBA, 0x98, 0x76, 0x54, 0x32, 0x11])
    writeInt64
    readInt64
    (-0x0123456789ABCDEF :: Int64)
  assertCodec
    "Float"
    (ByteString.pack [0x3F, 0x80, 0x00, 0x00])
    writeFloat
    readFloat
    (1.0 :: Float)
  assertCodec
    "Double"
    (ByteString.pack [0xC0, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00])
    writeDouble
    readDouble
    (-2.5 :: Double)

testBoolCodec :: IO ()
testBoolCodec = do
  assertCodec "False" (ByteString.pack [0]) writeBool readBool False
  assertCodec "True" (ByteString.pack [1]) writeBool readBool True

testTextCodec :: IO ()
testTextCodec = do
  let value = Text.pack "\x96EA \x1F680 e\x0301\NUL"
      suffix = Text.pack "\x03BB"
      expected = ByteString.pack [0, 0, 0, 13] <> encodeUtf8 value
  assertEqual "Text byte length and UTF-8" expected (runEncoder (writeText value))
  decoded <- runDecoder ((,) <$> readText <*> readText) (runEncoder (writeText value <> writeText suffix))
  assertEqual "nested Unicode Text" (value, suffix) decoded

testBytesCodec :: IO ()
testBytesCodec = do
  let value = ByteString.pack [0x00, 0x80, 0xFF]
      expected = ByteString.pack [0, 0, 0, 3, 0x00, 0x80, 0xFF]
  assertCodec "bytes" expected writeBytes readBytes value

testMaybeAndListCodecs :: IO ()
testMaybeAndListCodecs = do
  let value = Just [0x1234, -2] :: Maybe [Int16]
      writeValue = writeMaybe (writeList writeInt16)
      readValue = readMaybe (readList readInt16)
      expected = ByteString.pack [1, 0, 0, 0, 2, 0x12, 0x34, 0xFF, 0xFE]
  assertCodec "Maybe list" expected writeValue readValue value
  assertCodec "Nothing" (ByteString.pack [0]) (writeMaybe writeWord8) (readMaybe readWord8) Nothing
  assertCodec "empty list" (ByteString.pack [0, 0, 0, 0]) (writeList writeWord8) (readList readWord8) []

testMapAndSetCodecs :: IO ()
testMapAndSetCodecs = do
  let mapValue = Map.fromList [(-2, 0x0405), (1, 0x0203)] :: Map Int8 Word16
      mapBytes = ByteString.pack [0, 0, 0, 2, 0xFE, 0x04, 0x05, 0x01, 0x02, 0x03]
      emptyMap = Map.empty :: Map Int8 Word16
      setValue = Set.fromList [0x1234, -2] :: Set Int16
      setBytes = ByteString.pack [0, 0, 0, 2, 0xFF, 0xFE, 0x12, 0x34]
      emptySet = Set.empty :: Set Int16
  assertCodec "map" mapBytes (writeMap writeInt8 writeWord16) (readMap readInt8 readWord16) mapValue
  assertCodec
    "empty map"
    (ByteString.pack [0, 0, 0, 0])
    (writeMap writeInt8 writeWord16)
    (readMap readInt8 readWord16)
    emptyMap
  assertCodec "set" setBytes (writeSet writeInt16) (readSet readInt16) setValue
  assertCodec
    "empty set"
    (ByteString.pack [0, 0, 0, 0])
    (writeSet writeInt16)
    (readSet readInt16)
    emptySet

testTimeCodecs :: IO ()
testTimeCodecs = do
  let postEpoch = Timestamp 0x0123456789ABCDEF 999999999
      postEpochBytes =
        ByteString.pack
          [0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF, 0x3B, 0x9A, 0xC9, 0xFF]
      preEpoch = Timestamp (-0x0123456789ABCDEF) 1
      preEpochBytes =
        ByteString.pack
          [0xFE, 0xDC, 0xBA, 0x98, 0x76, 0x54, 0x32, 0x11, 0x00, 0x00, 0x00, 0x01]
      duration = Duration 0xFEDCBA9876543210 500000000
      durationBytes =
        ByteString.pack
          [0xFE, 0xDC, 0xBA, 0x98, 0x76, 0x54, 0x32, 0x10, 0x1D, 0xCD, 0x65, 0x00]
      malformedTimestamp = writeInt64 0 <> writeWord32 1000000000
      malformedDuration = writeWord64 0 <> writeWord32 1000000000
  assertCodec "post-epoch Timestamp" postEpochBytes writeTimestamp readTimestamp postEpoch
  assertCodec "pre-epoch Timestamp" preEpochBytes writeTimestamp readTimestamp preEpoch
  assertCodec "Duration" durationBytes writeDuration readDuration duration
  assertEqual "Timestamp ordering" True (preEpoch < postEpoch)
  assertEqual "Duration ordering" True (Duration 0 999999999 < Duration 1 0)
  assertEncoderFails "Timestamp malformed nanoseconds" (writeTimestamp (Timestamp 0 1000000000))
  assertEncoderFails "Duration malformed nanoseconds" (writeDuration (Duration 0 1000000000))
  assertDecoderFails "Timestamp malformed nanoseconds" readTimestamp (runEncoder malformedTimestamp)
  assertDecoderFails "Duration malformed nanoseconds" readDuration (runEncoder malformedDuration)

testDecoderFailures :: IO ()
testDecoderFailures = do
  assertDecoderFails
    "truncated integer"
    readWord64
    (ByteString.pack [0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD])
  assertDecoderFails
    "truncated bytes"
    readBytes
    (ByteString.pack [0, 0, 0, 3, 0x01, 0x02])
  assertDecoderFails "invalid Bool tag" readBool (ByteString.pack [2])
  assertDecoderFails "invalid Maybe tag" (readMaybe readWord8) (ByteString.pack [2])
  assertDecoderFails "negative map count" (readMap readWord8 readWord8) (ByteString.pack [0xFF, 0xFF, 0xFF, 0xFF])
  assertDecoderFails "negative set count" (readSet readWord8) (ByteString.pack [0xFF, 0xFF, 0xFF, 0xFF])
  assertDecoderFails
    "invalid UTF-8"
    readText
    (ByteString.pack [0, 0, 0, 2, 0xC3, 0x28])
  assertDecoderFails "trailing bytes" readWord8 (ByteString.pack [1, 2])
